//! The report of a task, in the one spelling both seams hand it over.
//!
//! `marsrs-stn` is asked for the report by two hosts — the JNI seam
//! (`mars/stn/jni/com_tencent_mars_stn_StnLogic_C2Java.cc`) and the C ABI
//! (`marsrs-ffi`'s `stn` feature) — and both hand the same document to the app,
//! because the keys and their order are the C++'s: this is the module that
//! keeps them from drifting apart.

use crate::TaskProfile;

/// `C2Java_ReportTaskProfile` — the report, as the json the app parses.
///
/// The C++ writes it field by field into an `XMessage`; the json is the wire
/// format between the two halves, so the keys and their order are the C++'s.
/// Like the C++, no string is escaped: a cgi with a quote in it is the app's
/// own problem.
pub fn task_profile_json(profile: &TaskProfile) -> String {
    let connections: Vec<String> = profile
        .history
        .iter()
        .map(|transfer| {
            let connect = &transfer.connect_profile;
            format!(
                concat!(
                    "{{\"startTime\":{},\"dnsTime\":{},\"dnsEndTime\":{},\"connTime\":{},",
                    "\"connErrCode\":{},\"tryIPCount\":{},\"ip\":\"{}\",\"port\":{},",
                    "\"host\":\"{}\",\"ipType\":{},\"disconnTime\":{},",
                    "\"disconnErrType\":{},\"disconnErrCode\":{}}}"
                ),
                connect.start_time,
                connect.dns_time,
                connect.dns_endtime,
                connect.conn_time,
                connect.conn_errcode,
                connect.tryip_count,
                connect.ip,
                connect.port,
                connect.host,
                connect.ip_type as i32,
                connect.disconn_time,
                connect.disconn_errtype as i32,
                connect.disconn_errcode,
            )
        })
        .collect();

    format!(
        concat!(
            "{{\"taskId\":{},\"cmdId\":{},\"cgi\":\"{}\",\"startTaskTime\":{},",
            "\"endTaskTime\":{},\"dyntimeStatus\":{},\"errCode\":{},\"errType\":{},",
            "\"channelSelect\":{},\"historyNetLinkers\":[{}]}}"
        ),
        profile.task.taskid,
        profile.task.cmdid,
        profile.task.cgi,
        profile.start_task_time,
        profile.end_task_time,
        profile.current_dyntime_status as i32,
        profile.err_code,
        profile.err_type as i32,
        profile.link_type,
        connections.join(","),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DynamicTimeoutStatus, ErrCmdType, IpSourceType, PrepareProfile, Task, TransferProfile,
    };

    /// A task that ran: one try in its history, which is the
    /// `historyNetLinkers` of the report. Only the readings the json writes
    /// are set.
    fn report() -> TaskProfile {
        let task = Task::new(7, 8);
        let mut profile = TaskProfile::new_at(
            0,
            Task {
                cgi: "cgi".to_owned(),
                ..task.clone()
            },
            PrepareProfile {
                start_task_call_time: 0,
                begin_process_hosts_time: 0,
                end_process_hosts_time: 0,
            },
        );
        profile.end_task_time = 200;
        profile.err_code = -15;
        profile.err_type = ErrCmdType::Local;
        profile.current_dyntime_status = DynamicTimeoutStatus::Evaluating;

        let mut transfer = TransferProfile::new(task);
        transfer.connect_profile.start_time = 10;
        transfer.connect_profile.dns_time = 11;
        transfer.connect_profile.dns_endtime = 12;
        transfer.connect_profile.conn_time = 13;
        transfer.connect_profile.conn_errcode = -1;
        transfer.connect_profile.tryip_count = 2;
        transfer.connect_profile.ip = "10.0.0.1".to_owned();
        transfer.connect_profile.port = 80;
        transfer.connect_profile.host = "host".to_owned();
        transfer.connect_profile.ip_type = IpSourceType::Dns;
        transfer.connect_profile.disconn_time = 14;
        transfer.connect_profile.disconn_errtype = ErrCmdType::Local;
        transfer.connect_profile.disconn_errcode = 15;
        profile.history.push(transfer);
        profile
    }

    #[test]
    fn the_report_is_the_json_the_cpp_writes() {
        assert_eq!(
            task_profile_json(&report()),
            concat!(
                r#"{"taskId":7,"cmdId":8,"cgi":"cgi","startTaskTime":0,"#,
                r#""endTaskTime":200,"dyntimeStatus":1,"errCode":-15,"errType":9,"#,
                r#""channelSelect":0,"historyNetLinkers":[{"startTime":10,"dnsTime":11,"#,
                r#""dnsEndTime":12,"connTime":13,"connErrCode":-1,"tryIPCount":2,"#,
                r#""ip":"10.0.0.1","port":80,"host":"host","ipType":2,"#,
                r#""disconnTime":14,"disconnErrType":9,"disconnErrCode":15}]}"#
            )
        );
    }

    /// A task that never ran has no history, which is an empty list and not a
    /// json `null`: the app parses it without asking which it got.
    #[test]
    fn a_task_that_never_ran_reports_an_empty_history() {
        let profile = TaskProfile::new_at(
            0,
            Task::new(1, 0),
            PrepareProfile {
                start_task_call_time: 0,
                begin_process_hosts_time: 0,
                end_process_hosts_time: 0,
            },
        );
        assert!(task_profile_json(&profile).contains(r#""historyNetLinkers":[]}"#));
    }
}
