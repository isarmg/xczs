//! Current product-owned access-log format grammar and bounded parser.
use serde_json::{Value, json};

pub(crate) const MAX_LOG_FORMAT_BYTES: usize = 4096;
pub(crate) const MAX_LOG_FORMAT_ELEMENTS: usize = 128;

macro_rules! variables {
    ($($variant:ident => $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub(crate) enum Variable { $($variant),* }
        impl Variable {
            pub(crate) fn name(self) -> &'static str { match self { $(Self::$variant => $name),* } }
        }
        pub(crate) fn named(name: &str) -> Option<Variable> {
            match name { $($name => Some(Variable::$variant),)* _ => None }
        }
        pub(crate) const VARIABLES: &[&str] = &[$($name),*];
    };
}
variables! {
    Request => "request", RequestMethod => "request_method", RequestUri => "request_uri",
    RemoteUser => "remote_user", TimeUtc => "time_utc", TimeIso8601 => "time_iso8601",
    Msec => "msec", Status => "status", OperationId => "operation_id",
    OperationState => "operation_state", LogLevel => "log_level",
    RequestId => "request_id",
}

#[derive(Debug, PartialEq)]
pub(crate) enum Token {
    Literal(String),
    Variable(Variable),
    Header(String),
}

pub(crate) fn parse(input: &str) -> Result<Vec<Token>, &'static str> {
    if input.len() > MAX_LOG_FORMAT_BYTES {
        return Err("HTTP log format exceeds the 4096-byte limit");
    }
    let mut result = Vec::new();
    let mut position = 0;
    while position < input.len() {
        let next = input[position..]
            .find('$')
            .map(|offset| position + offset)
            .unwrap_or(input.len());
        if next > position {
            result.push(Token::Literal(input[position..next].to_owned()));
        }
        if next == input.len() {
            break;
        }
        let start = next + 1;
        let end = input[start..]
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map(|(offset, _)| start + offset)
            .unwrap_or(input.len());
        let name = &input[start..end];
        if let Some(variable) = named(name) {
            result.push(Token::Variable(variable));
        } else if let Some(header) = name.strip_prefix("http_") {
            if header.is_empty()
                || !header
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err("Invalid HTTP request header log variable");
            }
            result.push(Token::Header(header.to_ascii_lowercase().replace('_', "-")));
        } else {
            return Err("Unknown HTTP log variable");
        }
        position = end;
        if result.len() > MAX_LOG_FORMAT_ELEMENTS {
            return Err("HTTP log format exceeds the 128-element limit");
        }
    }
    if result.len() > MAX_LOG_FORMAT_ELEMENTS {
        return Err("HTTP log format exceeds the 128-element limit");
    }
    Ok(result)
}

pub(crate) fn rule() -> Value {
    json!({"kind":"token-template","fields":["/log_format"],"arguments":{
        "maximum_bytes":MAX_LOG_FORMAT_BYTES,"maximum_tokens":MAX_LOG_FORMAT_ELEMENTS,"marker":"$",
        "variables":VARIABLES,"header_prefix":"http_","header_name_encoding":"ascii-lowercase-underscore-to-hyphen"}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjacent_variables_are_tokens_and_unknown_variables_cannot_hide_before_another_marker() {
        assert_eq!(
            parse("$status$request").unwrap(),
            [
                Token::Variable(Variable::Status),
                Token::Variable(Variable::Request)
            ]
        );
        assert!(parse("$removed$status").is_err());
        assert!(parse("$$status").is_err());
        assert_eq!(
            parse("$http_X_Request_ID").unwrap(),
            [Token::Header("x-request-id".into())]
        );
        assert!(parse("$http_").is_err());
        assert!(parse("$http_文").is_err());
    }
    #[test]
    fn limits_count_utf8_bytes_and_actual_render_tokens() {
        assert!(parse(&"文".repeat(1365)).is_ok());
        assert!(parse(&"文".repeat(1366)).is_err());
        assert_eq!(parse(&"$status".repeat(128)).unwrap().len(), 128);
        assert!(parse(&"$status".repeat(129)).is_err());
        assert_eq!(parse(&"$status ".repeat(64)).unwrap().len(), 128);
        assert!(parse(&"$status ".repeat(65)).is_err());
    }
}
