#[derive(Clone, Debug)]
pub enum TuiCommand {
    Start { id: String },
    Stop { id: String },
    Restart { id: String },
    Kill { id: String },
    Toggle { id: String },
    ClearLogs { id: String },
    Exec { id: String, cmd: Vec<String> },
}

impl TuiCommand {
    pub async fn execute(self, backend: &crate::RuntimeBackend) {
        match self {
            TuiCommand::Start { id } => backend.send_start(id).await,
            TuiCommand::Stop { id } => backend.send_stop(id).await,
            TuiCommand::Restart { id } => backend.send_restart(id).await,
            TuiCommand::Kill { id } => backend.send_kill(id).await,
            TuiCommand::Toggle { id } => backend.send_toggle(id).await,
            TuiCommand::ClearLogs { id } => backend.send_clear_logs(id).await,
            TuiCommand::Exec { id, cmd } => backend.send_exec(id, cmd).await,
        }
    }
}

pub fn parse_command(input: &str, service_ids: &[String]) -> Result<Vec<TuiCommand>, String> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Empty command".into());
    }

    let cmd = parts[0].to_lowercase();
    let arg1 = parts.get(1).copied();

    let exists = |id: &str| service_ids.iter().any(|s| s == id);

    let expand_ids = |arg: Option<&str>| -> Result<Vec<String>, String> {
        match arg {
            Some("all") => Ok(service_ids.to_vec()),
            Some(id) if exists(id) => Ok(vec![id.to_string()]),
            Some(id) => Err(format!("Unknown service: {id}")),
            None => Err("Missing target (service id or 'all')".into()),
        }
    };

    match cmd.as_str() {
        "up" | "start" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Start { id })
            .collect()),

        "down" | "stop" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Stop { id })
            .collect()),

        "restart" | "rs" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Restart { id })
            .collect()),

        "toggle" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Toggle { id })
            .collect()),

        "kill" | "k" => Ok(expand_ids(arg1)?
            .into_iter()
            .map(|id| TuiCommand::Kill { id })
            .collect()),

        "clear" | "cl" => {
            let target = if arg1 == Some("logs") {
                parts.get(2).copied().unwrap_or("all")
            } else {
                arg1.unwrap_or("all")
            };
            Ok(expand_ids(Some(target))?
                .into_iter()
                .map(|id| TuiCommand::ClearLogs { id })
                .collect())
        }

        "exec" | "run" => {
            let svc = arg1.ok_or("Usage: exec <service> <cmd...>")?;
            if !exists(svc) {
                return Err(format!("Unknown service: {svc}"));
            }
            let cmd_parts = parts.get(2..).unwrap_or(&[]);
            if cmd_parts.is_empty() {
                return Err("Usage: exec <service> <cmd...>".into());
            }
            Ok(vec![TuiCommand::Exec {
                id: svc.to_string(),
                cmd: cmd_parts.iter().map(|s| s.to_string()).collect(),
            }])
        }

        _ => Err(format!(
            "Unknown command: {cmd}\nTry: up/down/restart/toggle/kill/clear/exec"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        vec!["api".into(), "worker".into(), "db".into()]
    }

    #[test]
    fn parses_start_with_alias() {
        let out = parse_command("up api", &ids()).unwrap();
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], TuiCommand::Start { ref id } if id == "api"));
    }

    #[test]
    fn expands_all() {
        let out = parse_command("restart all", &ids()).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn rejects_unknown_service() {
        let err = parse_command("stop nope", &ids()).unwrap_err();
        assert!(err.contains("Unknown service"), "got: {err}");
    }

    #[test]
    fn clear_with_logs_keyword() {
        let out = parse_command("clear logs api", &ids()).unwrap();
        assert!(matches!(out[0], TuiCommand::ClearLogs { ref id } if id == "api"));
    }

    #[test]
    fn exec_requires_command() {
        let err = parse_command("exec api", &ids()).unwrap_err();
        assert!(err.contains("Usage"), "got: {err}");
    }

    #[test]
    fn rejects_empty_input() {
        let err = parse_command("", &ids()).unwrap_err();
        assert!(err.contains("Empty"), "got: {err}");
    }
}
