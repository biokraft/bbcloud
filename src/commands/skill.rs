use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::skill::{self, Action, Agent};
use serde::Serialize;

#[derive(Serialize)]
struct OutcomeRow {
    path: String,
    agent: String,
    action: String,
}

#[derive(Serialize)]
struct StatusRowJson {
    path: String,
    agent: String,
    state: String,
}

#[derive(Serialize)]
struct UninstallRowJson {
    path: String,
    removed: bool,
}

pub fn install(format: Format, agent: Option<&str>, global: bool, force: bool) -> Result<()> {
    let root = if global {
        home_dir()?
    } else {
        std::env::current_dir().map_err(BbError::Io)?
    };

    let agents = match agent {
        Some("all") => skill::Agent::all().to_vec(),
        Some("agents") => vec![Agent::Agents],
        Some("claude") => vec![Agent::Claude],
        Some(other) => {
            return Err(BbError::Config(format!(
                "unknown agent `{other}` — expected agents, claude or all"
            )))
        }
        None => {
            let detected = skill::detect_agents(&root);
            if detected.is_empty() {
                // `.agents/skills/` is the portable location Codex, Cursor and
                // OpenCode all read, so it is the safe default.
                if !format.is_json() {
                    output::info(
                        "no agent directory found — installing to .agents/skills/, which Codex, Cursor and OpenCode read",
                    );
                }
                vec![Agent::Agents]
            } else {
                detected
            }
        }
    };

    let outcomes = skill::install(&root, &agents, force)?;
    let rows: Vec<OutcomeRow> = outcomes
        .iter()
        .map(|o| OutcomeRow {
            path: o.path.display().to_string(),
            agent: o.agent.clone(),
            action: o.action.as_str().to_string(),
        })
        .collect();

    match format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => {
            for row in &rows {
                let line = format!("{} {}", row.action, row.path);
                match row.action.as_str() {
                    "unchanged" => output::info(&line),
                    "skipped_modified" => output::warn(&line),
                    _ => output::success(&line),
                }
            }
        }
    }

    // A refusal is an error the user must act on, so it sets the exit code —
    // after the report, so they can see which paths were fine.
    if outcomes.iter().any(|o| o.action == Action::SkippedModified) {
        return Err(BbError::Config(
            "some skills were edited locally and were left alone — pass --force to overwrite"
                .into(),
        ));
    }
    Ok(())
}

pub fn status(format: Format) -> Result<()> {
    let (rows, warning) = skill::status();
    if let Some(warning) = warning {
        output::warn(&warning);
    }

    match format {
        Format::Json => {
            let json_rows: Vec<StatusRowJson> = rows
                .iter()
                .map(|r| StatusRowJson {
                    path: r.path.display().to_string(),
                    agent: r.agent.clone(),
                    state: r.state.as_str().to_string(),
                })
                .collect();
            output::print_json(&json_rows)?;
        }
        Format::Human => {
            let table_rows: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    vec![
                        r.path.display().to_string(),
                        r.agent.clone(),
                        r.state.as_str().to_string(),
                    ]
                })
                .collect();
            output::print_table(&["PATH", "AGENT", "STATE"], table_rows);
        }
    }
    Ok(())
}

pub fn uninstall(format: Format, global: bool, force: bool) -> Result<()> {
    let root = if global {
        home_dir()?
    } else {
        std::env::current_dir().map_err(BbError::Io)?
    };

    let results = skill::uninstall(Some(&root), force)?;

    match format {
        Format::Json => {
            let json_rows: Vec<UninstallRowJson> = results
                .iter()
                .map(|(path, removed)| UninstallRowJson {
                    path: path.display().to_string(),
                    removed: *removed,
                })
                .collect();
            output::print_json(&json_rows)?;
        }
        Format::Human => {
            if results.is_empty() {
                output::info("nothing to uninstall");
            }
            for (path, removed) in &results {
                if *removed {
                    output::success(&format!("removed {}", path.display()));
                } else {
                    output::warn(&format!(
                        "{} was edited locally — left alone (pass --force to remove)",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn home_dir() -> Result<std::path::PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| BbError::Config("HOME is not set, so --global has no target".into()))
}
