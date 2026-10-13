use super::*;

#[tokio::test]
async fn existing_credentials_preserve_stable_identity_after_rename() {
    use xcss::admin_core::{AdministratorManagementContext, LoginContext};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("accounts");
    let directory = xcss::fs_safety::PrivateDirectory::create(&path).unwrap();
    // Runtime must never create an administrator from an omitted auth setting.
    assert!(administrator_service_with_directory(directory).is_err());
    assert!(!path.join("administrator-accounts.json").exists());
    initialize_administrator(
        xcss::fs_safety::PrivateDirectory::open_existing(&path).unwrap(),
        "admin",
        "initial-password",
    )
    .unwrap();
    let first = administrator_service_with_directory(
        xcss::fs_safety::PrivateDirectory::open_existing(&path).unwrap(),
    )
    .unwrap();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64
    };
    let login_context = LoginContext {
        source: "127.0.0.1".into(),
        request_id: None,
        now_micros: now(),
    };
    let login = first
        .login("admin", "initial-password", &login_context)
        .await
        .unwrap();
    let context = AdministratorManagementContext {
        identity: first
            .authenticate_session(&login.session_token, now())
            .await
            .unwrap(),
        request_id: None,
        now_micros: now(),
    };
    first
        .update_own_account(
            &context,
            "renamed",
            "initial-password",
            Some("updated-password"),
        )
        .await
        .unwrap();
    drop(first);
    let saved = std::fs::read(path.join("administrator-accounts.json")).unwrap();
    let second = administrator_service_with_directory(
        xcss::fs_safety::PrivateDirectory::open_existing(&path).unwrap(),
    )
    .unwrap();
    assert_eq!(
        saved,
        std::fs::read(path.join("administrator-accounts.json")).unwrap()
    );
    let login = second
        .login(
            "renamed",
            "updated-password",
            &LoginContext {
                now_micros: now(),
                ..login_context
            },
        )
        .await
        .unwrap();
    assert_eq!(login.administrator.user_id.as_str(), "admin");
}
