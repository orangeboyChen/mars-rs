//! The JSON a finished diagnosis is handed over as.
//!
//! `SdtManagerJniCallback::ReportNetCheckResult()` — and now the C ABI too —
//! hands a diagnosis to the app as one JSON document, field for field the
//! C++'s. It is here and not in `marsrs-jni` or `marsrs-ffi` because the fields are
//! [`CheckResultProfile`]'s, and both seams that hand a report over have to
//! spell them the same way.

use crate::netchecker_profile::CheckResultProfile;

/// A JSON string field.
///
/// The C++ writes `iter->ip` and the rest straight into the document, so a
/// quote or a newline in a domain name — and the domain names come from the
/// caller — used to break the whole report.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for char in value.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if control < ' ' => out.push_str(&format!("\\u{:04x}", control as u32)),
            _ => out.push(char),
        }
    }
    out.push('"');
    out
}

/// The report the app gets: `{"details":[ … ]}`, one object per check, with the
/// string fields escaped.
pub fn report_json(check_results: &[CheckResultProfile]) -> String {
    let mut json = String::from("{\"details\":[");
    for (index, result) in check_results.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"detectType\":{},\"errorCode\":{},\"networkType\":{},\"detectIP\":{},\"port\":{},\"conntime\":{},\"rtt\":{},\"rttStr\":{},\"httpStatusCode\":{},\"pingCheckCount\":{},\"pingLossRate\":{},\"dnsDomain\":{},\"localDns\":{},\"dnsIP1\":{},\"dnsIP2\":{}}}",
            result.netcheck_type,
            result.error_code,
            result.network_type,
            json_string(&result.ip),
            result.port,
            result.conntime,
            result.rtt,
            json_string(&result.rtt_str),
            result.status_code,
            result.checkcount,
            json_string(&result.loss_rate),
            json_string(&result.domain_name),
            json_string(&result.local_dns),
            json_string(&result.ip1),
            json_string(&result.ip2),
        ));
    }
    json.push_str("]}");
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netchecker_profile::CheckResultProfile;

    fn profile() -> CheckResultProfile {
        CheckResultProfile {
            ip: "1.2.3.4".to_owned(),
            rtt_str: "12ms".to_owned(),
            checkcount: 3,
            ..CheckResultProfile::default()
        }
    }

    #[test]
    fn one_object_per_check() {
        let json = report_json(&[profile(), profile()]);
        assert!(json.starts_with("{\"details\":["), "{json}");
        assert!(json.ends_with("]}"), "{json}");
        assert_eq!(
            json.matches("\"detectIP\":\"1.2.3.4\"").count(),
            2,
            "{json}"
        );
        assert!(json.contains("\"rttStr\":\"12ms\""), "{json}");
    }

    #[test]
    fn no_results_is_an_empty_list() {
        assert_eq!(report_json(&[]), "{\"details\":[]}");
    }

    #[test]
    fn the_string_fields_are_escaped() {
        let profile = CheckResultProfile {
            domain_name: "a\"b\\c\nd".to_owned(),
            ..profile()
        };
        let json = report_json(&[profile]);
        assert!(json.contains("\\\""), "the quote is escaped: {json}");
        assert!(json.contains("\\\\"), "the backslash is escaped: {json}");
        assert!(json.contains("\\n"), "the newline is escaped: {json}");
    }
}
