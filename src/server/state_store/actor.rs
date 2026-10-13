use super::*;

pub(super) struct ActorRuntime {
    pub(super) path: PathBuf,
    pub(super) temporary_root: PathBuf,
    pub(super) root: RootIdentity,
    pub(super) limits: Limits,
    pub(super) command_receiver: Receiver<Command>,
    pub(super) control_receiver: Receiver<ControlCommand>,
    pub(super) channels: ActorChannels,
    pub(super) healthy: Arc<AtomicBool>,
    pub(super) ready: SyncSender<Result<()>>,
    pub(super) require_administrator: bool,
}

struct HealthOnExit(Arc<AtomicBool>);

impl Drop for HealthOnExit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) fn run(runtime: ActorRuntime) {
    let ActorRuntime {
        path,
        temporary_root,
        root,
        limits,
        command_receiver,
        control_receiver,
        channels,
        healthy,
        ready,
        require_administrator,
    } = runtime;
    let _health_on_exit = HealthOnExit(healthy.clone());
    let initialized = (|| {
        let mut clock = StoreClock::new()?;
        let recovery_now = clock.now_ms()?;
        let connection = database::open_initialized_connection(
            &path,
            root,
            limits.ttl_ms,
            limits.upload_ttl_ms,
            recovery_now,
            &temporary_root,
            require_administrator,
        )?;
        Ok::<_, anyhow::Error>((connection, clock))
    })()
    .with_context(|| "Failed to initialize the state database");

    let (connection, clock) = match initialized {
        Ok(initialized) => initialized,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };

    healthy.store(true, Ordering::Release);
    if ready.send(Ok(())).is_err() {
        let _ = xcss::sqlite::block_on_sqlite_connection(connection.close());
        return;
    }

    StoreWorker {
        connection,
        clock,
        limits,
        deferred_abandons: VecDeque::new(),
        #[cfg(test)]
        execution_counts: ActorExecutionCounts::default(),
    }
    .run(command_receiver, control_receiver, channels);
}

impl StoreWorker {
    fn run(
        mut self,
        command_receiver: Receiver<Command>,
        control_receiver: Receiver<ControlCommand>,
        channels: ActorChannels,
    ) {
        loop {
            if self.process_controls(&control_receiver) {
                break;
            }
            let command = match command_receiver.recv() {
                Ok(command) => command,
                Err(_) => break,
            };
            match command {
                Command::Wake => continue,
                Command::ProbeReadiness { reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    // A failed probe describes this command, not the actor's
                    // lifecycle. A later probe may succeed after a transient
                    // lock, I/O, or capacity condition clears.
                    let _ = reply.send(self.probe_readiness());
                }
                Command::Begin {
                    key,
                    fingerprint,
                    reply,
                } => {
                    // If cancellation is already visible, no reservation was
                    // transferred and there is nothing to clean up. A close
                    // racing this check is still handled by the send failure
                    // and BeginEnvelope cleanup paths below.
                    if reply.is_closed() {
                        continue;
                    }
                    #[cfg(test)]
                    {
                        self.execution_counts.begin += 1;
                    }
                    let result = self
                        .begin_operation(key, fingerprint)
                        .map(|begin| BeginEnvelope::new(begin, &channels, key));
                    match reply.send(result) {
                        Ok(()) | Err(Err(_)) => {}
                        Err(Ok(mut envelope)) => {
                            if let Some(lease) = envelope.reservation() {
                                // Cancellation before delivery leaves a
                                // Reserved row that must be removed. A single
                                // SQLite failure must not kill the actor, so a
                                // failed cleanup is retained for a later
                                // command boundary.
                                envelope.disarm_cleanup();
                                if let Err(error) = self.abandon_operation(key, lease) {
                                    log::error!(
                                        "State store reservation cleanup was deferred: {error:#}"
                                    );
                                    self.defer_abandon(key, lease);
                                }
                            }
                        }
                    }
                }
                Command::Status { key, reply } => {
                    if reply.is_closed() {
                        // Expiry reclamation is incidental to Status. Every
                        // later live Status or Begin performs the same purge,
                        // so cancellation does not leave required cleanup.
                        continue;
                    }
                    #[cfg(test)]
                    {
                        self.execution_counts.status += 1;
                    }
                    let _ = reply.send(self.operation_status(key));
                }
                Command::MarkCommitStarted { key, lease, reply } => {
                    let _ = reply.send(self.mark_operation_commit_started(key, lease));
                }
                Command::Complete {
                    key,
                    lease,
                    outcome,
                    reply,
                } => {
                    #[cfg(test)]
                    {
                        self.execution_counts.complete += 1;
                    }
                    let _ = reply.send(self.complete_operation(key, lease, &outcome));
                }
                Command::SaveUploadSession {
                    session,
                    ttl_ms,
                    reply,
                } => {
                    let _ = reply.send(self.save_upload_session(&session, ttl_ms));
                }
                Command::LoadUploadSession { key, reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(xcss::sqlite::block_on_sqlite_connection(async {
                        self.reset_deadline().await?;
                        super::upload::load_upload_session(&mut self.connection, key).await
                    }));
                }
                Command::ListUploadSessionsPageBlocking {
                    after,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(self.upload_sessions_page(after, limit));
                }
                Command::ListUploadSessionsPage {
                    after,
                    limit,
                    reply,
                } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.upload_sessions_page(after, limit));
                }
                Command::RejectUploadSession {
                    key,
                    target_path,
                    stage_path,
                    ttl_ms,
                    reply,
                } => {
                    let _ = reply.send(self.reject_upload_session(
                        key,
                        &target_path,
                        &stage_path,
                        ttl_ms,
                    ));
                }
                Command::ListExpiredUploadSessions {
                    after,
                    limit,
                    reply,
                } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.expired_upload_sessions(after, limit));
                }
                Command::MatchExpiredUploadSession { expected, reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.expired_upload_session_matches(&expected));
                }
                Command::RemoveUploadSessionIfMatches { expected, reply } => {
                    let _ = reply.send(self.remove_upload_session_if_matches(&expected));
                }
                Command::RemoveExpiredUploadSessionIfMatches { expected, reply } => {
                    let _ = reply.send(self.remove_expired_upload_session_if_matches(&expected));
                }
                Command::PreparePurgeJob { job, reply } => {
                    let _ = reply.send(self.prepare_purge_job(&job));
                }
                Command::LoadPurgeJob { key, reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(xcss::sqlite::block_on_sqlite_connection(async {
                        self.reset_deadline().await?;
                        super::purge::load_purge_job(&mut self.connection, key).await
                    }));
                }
                Command::ListPreparedPurgeJobs { limit, reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.prepared_purge_jobs(limit));
                }
                Command::ListPurgeJobs { limit, reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.purge_jobs(limit));
                }
                Command::IsStatePathBoundBlocking { path, reply } => {
                    let _ = reply.send(self.state_path_is_bound(&path));
                }
                Command::ListStateBlockingPaths {
                    after,
                    limit,
                    scan_lease,
                    reply,
                } => {
                    if reply.is_closed() {
                        drop(scan_lease);
                        continue;
                    }
                    let result = self.state_blocking_paths(after, limit);
                    drop(scan_lease);
                    let _ = reply.send(result);
                }
                Command::MarkPurgeJobReady {
                    key,
                    trash_revision,
                    reply,
                } => {
                    let _ = reply.send(self.mark_purge_job_ready(key, trash_revision));
                }
                Command::ClaimDuePurgeJob { reply } => {
                    let _ = reply.send(self.claim_due_purge_job());
                }
                Command::RetryPurgeJob {
                    key,
                    delay_ms,
                    reply,
                } => {
                    let _ = reply.send(self.retry_purge_job(key, delay_ms));
                }
                Command::CompletePurgeJob { key, reply } => {
                    let _ = reply.send(self.complete_purge_job(key));
                }
                Command::RemovePurgeJob { key, reply } => {
                    let _ = reply.send(self.remove_purge_job(key));
                }
                #[cfg(test)]
                Command::InspectPragmas { reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.inspect_pragmas());
                }
                #[cfg(test)]
                Command::InspectActorExecutionCounts { reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(Ok(self.execution_counts));
                }
                #[cfg(test)]
                Command::InjectSqlError { reply } => {
                    let result = xcss::sqlite::block_on_sqlite_connection(async {
                        sqlx::query("SELECT * FROM __xczs_missing_test_table")
                            .execute(&mut self.connection)
                            .await
                            .context("Injected state-store SQL command failed")?;
                        Ok(())
                    });
                    let _ = reply.send(result);
                }
                #[cfg(test)]
                Command::SetQueryOnly { enabled, reply } => {
                    let result = xcss::sqlite::block_on_sqlite_connection(async {
                        let sql = if enabled {
                            "PRAGMA query_only=ON"
                        } else {
                            "PRAGMA query_only=OFF"
                        };
                        sqlx::query(sql)
                            .execute(&mut self.connection)
                            .await
                            .context("Failed to change SQLite query-only test mode")?;
                        Ok(())
                    });
                    let _ = reply.send(result);
                }
                #[cfg(test)]
                Command::Block { entered, release } => {
                    let _ = entered.send(());
                    let _ = release.recv();
                }
            }
        }
        if xcss::sqlite::block_on_sqlite_connection(self.connection.close()).is_err() {
            log::error!("The state database worker could not be closed");
        }
    }

    fn process_controls(&mut self, receiver: &Receiver<ControlCommand>) -> bool {
        self.retry_one_deferred_abandon();
        while let Ok(control) = receiver.try_recv() {
            match control {
                ControlCommand::Abandon { key, lease } => {
                    if let Err(error) = self.abandon_operation(key, lease) {
                        log::error!("State store control cleanup was deferred: {error:#}");
                        self.defer_abandon(key, lease);
                    }
                }
                ControlCommand::Shutdown => return true,
            }
        }
        false
    }

    fn defer_abandon(&mut self, key: OperationKey, lease: [u8; 16]) {
        if !self
            .deferred_abandons
            .iter()
            .any(|pending| *pending == (key, lease))
        {
            self.deferred_abandons.push_back((key, lease));
        }
    }

    fn retry_one_deferred_abandon(&mut self) {
        let Some((key, lease)) = self.deferred_abandons.pop_front() else {
            return;
        };
        if let Err(error) = self.abandon_operation(key, lease) {
            log::error!("State store deferred cleanup still cannot run: {error:#}");
            self.deferred_abandons.push_back((key, lease));
        }
    }

    pub(super) async fn reset_deadline(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        self.connection
            .lock_handle()
            .await?
            .set_progress_handler(1000, move || Instant::now() < deadline);
        Ok(())
    }

    fn probe_readiness(&mut self) -> Result<()> {
        xcss::sqlite::block_on_sqlite_connection(async {
            self.reset_deadline().await?;
            database::validate_product_metadata(&mut self.connection)
                .await
                .context("The state database product identity changed")?;
            let mut transaction = self
                .connection
                .begin_with("BEGIN IMMEDIATE")
                .await
                .context("Failed to begin the state database readiness write probe")?;
            let metadata_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM store_meta")
                .fetch_one(&mut *transaction)
                .await?;
            ensure!(metadata_rows >= 2, "State database metadata is incomplete");
            sqlx::query("INSERT INTO store_meta(key,value) VALUES('readiness-probe',X'00') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                .execute(&mut *transaction).await?;
            transaction
                .rollback()
                .await
                .context("Failed to roll back the state database readiness probe")?;
            Ok(())
        })
    }
}
