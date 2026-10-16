//! Upload envelopes are bound to an exact upload identifier and file length.
//! HTTP success alone never authorizes completion or destination overwrite.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Running,
    AwaitingConfirmation,
    Committed,
    Rejected,
    NotSeen,
    NotStarted,
    Unknown,
}
impl State {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "running" => Self::Running,
            "awaiting-confirmation" => Self::AwaitingConfirmation,
            "committed" => Self::Committed,
            "rejected" => Self::Rejected,
            "not-seen" => Self::NotSeen,
            "not-started" => Self::NotStarted,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Fresh,
    Resume,
    Checkpoint,
    Discard,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub state: State,
    pub length: Option<u64>,
    pub offset: Option<u64>,
}
pub fn unsigned(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 16
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|n| *n <= 9_007_199_254_740_991)
}
pub fn revision(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn classify(
    phase: Phase,
    status: u16,
    header: impl Fn(&str) -> Option<String>,
    expected_id: &str,
    length: u64,
) -> Option<Bound> {
    if header("X-Xczs-Upload-Id").as_deref() != Some(expected_id) {
        return None;
    }
    let state = State::parse(&header("X-Xczs-Operation-State")?)?;
    let raw_length = header("X-Xczs-Upload-Length");
    let raw_offset = header("X-Xczs-Upload-Offset");
    let bound_length = raw_length.as_deref().and_then(unsigned);
    let offset = raw_offset.as_deref().and_then(unsigned);
    if raw_length.is_some() && bound_length != Some(length) {
        return None;
    }
    if raw_offset.is_some() && offset.is_none_or(|n| n > length) {
        return None;
    }
    match state {
        State::Running | State::AwaitingConfirmation | State::Committed
            if bound_length.is_none() || offset.is_none() =>
        {
            return None;
        }
        State::Rejected | State::NotStarted if bound_length.is_none() => return None,
        State::NotSeen if raw_length.is_some() || raw_offset.is_some() => return None,
        _ => {}
    }
    let accepted = match (phase, state) {
        (Phase::Fresh, State::Committed) => matches!(status, 200 | 201),
        (Phase::Resume, State::Committed) => matches!(status, 200 | 204),
        (Phase::Fresh, State::Running) => matches!(status, 408 | 409),
        (Phase::Fresh, State::AwaitingConfirmation) => status == 409,
        (Phase::Resume, State::Running | State::AwaitingConfirmation) => {
            matches!(status, 408 | 409 | 413 | 500 | 507)
        }
        (Phase::Fresh | Phase::Resume, State::Rejected) => {
            matches!(status, 408 | 409 | 413 | 500 | 507)
        }
        (Phase::Fresh | Phase::Resume, State::NotSeen) => status == 404,
        (Phase::Fresh | Phase::Resume, State::NotStarted) => {
            matches!(status, 403 | 404 | 408 | 409 | 429 | 503)
        }
        (Phase::Fresh | Phase::Resume, State::Unknown) => matches!(status, 408 | 500 | 503 | 504),
        (Phase::Checkpoint, State::Running | State::Committed) => status == 200,
        (Phase::Checkpoint, State::AwaitingConfirmation | State::Rejected) => status == 409,
        (Phase::Checkpoint, State::NotSeen) => status == 404,
        (Phase::Checkpoint, State::Unknown) => matches!(status, 429 | 500 | 503),
        (Phase::Discard, State::Rejected) => status == 204,
        _ => false,
    };
    if !accepted
        || (state == State::Committed && offset != Some(length))
        || (phase == Phase::Discard && offset != Some(length))
        || (phase == Phase::Checkpoint
            && status == 429
            && (raw_length.is_some() || raw_offset.is_some()))
    {
        return None;
    }
    Some(Bound {
        state,
        length: bound_length,
        offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(
        id: &str,
        state: &str,
        length: Option<&str>,
        offset: Option<&str>,
    ) -> impl Fn(&str) -> Option<String> {
        let (id, state, length, offset) = (
            id.to_owned(),
            state.to_owned(),
            length.map(str::to_owned),
            offset.map(str::to_owned),
        );
        move |name| match name {
            "X-Xczs-Upload-Id" => Some(id.clone()),
            "X-Xczs-Operation-State" => Some(state.clone()),
            "X-Xczs-Upload-Length" => length.clone(),
            "X-Xczs-Upload-Offset" => offset.clone(),
            _ => None,
        }
    }
    #[test]
    fn success_requires_binding_and_complete_offset() {
        assert!(
            classify(
                Phase::Fresh,
                201,
                headers("a", "committed", Some("12"), Some("12")),
                "a",
                12
            )
            .is_some()
        );
        assert!(
            classify(
                Phase::Fresh,
                201,
                headers("b", "committed", Some("12"), Some("12")),
                "a",
                12
            )
            .is_none()
        );
        assert!(
            classify(
                Phase::Fresh,
                201,
                headers("a", "committed", Some("12"), Some("11")),
                "a",
                12
            )
            .is_none()
        );
        assert!(
            classify(
                Phase::Resume,
                201,
                headers("a", "committed", Some("12"), Some("12")),
                "a",
                12
            )
            .is_none()
        );
    }
    #[test]
    fn absence_and_discard_rules_fail_closed() {
        assert!(
            classify(
                Phase::Checkpoint,
                404,
                headers("a", "not-seen", None, None),
                "a",
                12
            )
            .is_some()
        );
        assert!(
            classify(
                Phase::Checkpoint,
                404,
                headers("a", "not-seen", Some("12"), None),
                "a",
                12
            )
            .is_none()
        );
        assert!(
            classify(
                Phase::Discard,
                204,
                headers("a", "rejected", Some("12"), Some("12")),
                "a",
                12
            )
            .is_some()
        );
        assert!(
            classify(
                Phase::Discard,
                204,
                headers("a", "rejected", Some("12"), Some("0")),
                "a",
                12
            )
            .is_none()
        );
        assert_eq!(unsigned("01"), None);
        assert_eq!(unsigned("9007199254740992"), None);
    }
    #[test]
    fn complete_http_status_phase_state_matrix_rejects_every_unlisted_combination() {
        let matrix: &[(Phase, State, &[u16])] = &[
            (Phase::Fresh, State::Running, &[408, 409]),
            (Phase::Fresh, State::AwaitingConfirmation, &[409]),
            (Phase::Fresh, State::Committed, &[200, 201]),
            (Phase::Fresh, State::Rejected, &[408, 409, 413, 500, 507]),
            (Phase::Fresh, State::NotSeen, &[404]),
            (
                Phase::Fresh,
                State::NotStarted,
                &[403, 404, 408, 409, 429, 503],
            ),
            (Phase::Fresh, State::Unknown, &[408, 500, 503, 504]),
            (Phase::Resume, State::Running, &[408, 409, 413, 500, 507]),
            (
                Phase::Resume,
                State::AwaitingConfirmation,
                &[408, 409, 413, 500, 507],
            ),
            (Phase::Resume, State::Committed, &[200, 204]),
            (Phase::Resume, State::Rejected, &[408, 409, 413, 500, 507]),
            (Phase::Resume, State::NotSeen, &[404]),
            (
                Phase::Resume,
                State::NotStarted,
                &[403, 404, 408, 409, 429, 503],
            ),
            (Phase::Resume, State::Unknown, &[408, 500, 503, 504]),
            (Phase::Checkpoint, State::Running, &[200]),
            (Phase::Checkpoint, State::AwaitingConfirmation, &[409]),
            (Phase::Checkpoint, State::Committed, &[200]),
            (Phase::Checkpoint, State::Rejected, &[409]),
            (Phase::Checkpoint, State::NotSeen, &[404]),
            (Phase::Checkpoint, State::NotStarted, &[]),
            (Phase::Checkpoint, State::Unknown, &[429, 500, 503]),
            (Phase::Discard, State::Running, &[]),
            (Phase::Discard, State::AwaitingConfirmation, &[]),
            (Phase::Discard, State::Committed, &[]),
            (Phase::Discard, State::Rejected, &[204]),
            (Phase::Discard, State::NotSeen, &[]),
            (Phase::Discard, State::NotStarted, &[]),
            (Phase::Discard, State::Unknown, &[]),
        ];
        for &(phase, state, statuses) in matrix {
            let state_name = serde_json::to_value(state)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            let absent = matches!(state, State::NotSeen | State::Unknown);
            for status in [
                200, 201, 202, 204, 206, 403, 404, 408, 409, 413, 418, 429, 500, 503, 504, 507,
            ] {
                let result = classify(
                    phase,
                    status,
                    headers(
                        "a",
                        &state_name,
                        if absent { None } else { Some("8") },
                        if absent { None } else { Some("8") },
                    ),
                    "a",
                    8,
                );
                assert_eq!(
                    result.is_some(),
                    statuses.contains(&status),
                    "{phase:?} {state:?} HTTP {status}"
                );
            }
        }
    }
}
