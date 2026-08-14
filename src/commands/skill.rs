use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::skill::{self, Action, Agent};
use serde::Serialize;

#[derive(Serialize)]
struct OutcomeRow {
    path: String,
    agent: String,
    skill: String,
    action: String,
}

#[derive(Serialize)]
struct StatusRowJson {
    path: String,
    agent: String,
    skill: String,
    state: String,
}

#[derive(Serialize)]
struct UninstallRowJson {
    path: String,
    skill: String,
    /// One of "removed", "refused_modified", "refused_unsafe_path", "absent" —
    /// kept as an explicit outcome string rather than a boolean so a consumer
    /// can tell "we refused to touch a local edit" apart from "there was
    /// nothing there to remove"; collapsing those into one `removed: false`
    /// used to make an honest "it just wasn't there" render as the same lie
    /// as a refusal, in both human text and this JSON.
    outcome: String,
}

/// The skills a command acts on. `None` means all of them; an unknown name is a
/// configuration error naming the valid ones, not a silent no-op.
fn wanted_skills(name: Option<&str>) -> Result<Vec<&'static skill::Skill>> {
    match name {
        None => Ok(skill::SKILLS.iter().collect()),
        Some(name) => skill::skill_by_name(name).map(|s| vec![s]).ok_or_else(|| {
            let valid: Vec<&str> = skill::SKILLS.iter().map(|s| s.name).collect();
            BbError::Config(format!(
                "unknown skill `{name}` — expected one of {}",
                valid.join(", ")
            ))
        }),
    }
}

/// The skills to act on, asking when the choice is genuinely open.
///
/// The prompt appears only when no `--skill` was given, `--all` was not passed,
/// the format is human, and stdin is a terminal. That last condition is
/// load-bearing three times over: the integration suite drives this binary with
/// piped stdin, CI has no terminal, and `auto_refresh_skills` runs before every
/// command's own logic — a prompt on any of those paths hangs rather than fails.
fn choose_skills(
    format: Format,
    skill_name: Option<&str>,
    all: bool,
) -> Result<Vec<&'static skill::Skill>> {
    let wanted = wanted_skills(skill_name)?;
    let interactive = skill_name.is_none()
        && !all
        && !format.is_json()
        && std::io::IsTerminal::is_terminal(&std::io::stdin());
    if !interactive {
        return Ok(wanted);
    }

    let options: Vec<String> = wanted
        .iter()
        .map(|s| format!("{} — {}", s.name, s.summary))
        .collect();
    // Everything preselected, so the fast path is one keypress and the
    // behaviour matches what this command did before the prompt existed.
    let defaults: Vec<usize> = (0..options.len()).collect();
    let picked = inquire::MultiSelect::new("Which skills should be installed?", options.clone())
        .with_default(&defaults)
        .prompt()
        .map_err(|_| BbError::Config("install cancelled — nothing was written".to_string()))?;

    Ok(pick_skills(&wanted, &options, &picked))
}

/// Maps the labels the user picked in the prompt back to the `&'static Skill`
/// values they came from. Pulled out of `choose_skills` because the prompt
/// itself only runs behind a terminal, so this is the part of that function a
/// test can actually reach.
fn pick_skills(
    wanted: &[&'static skill::Skill],
    options: &[String],
    picked: &[String],
) -> Vec<&'static skill::Skill> {
    options
        .iter()
        .enumerate()
        .filter(|(_, label)| picked.contains(label))
        .map(|(i, _)| wanted[i])
        .collect()
}

pub fn install(
    format: Format,
    agent: Option<&str>,
    global: bool,
    force: bool,
    skill_name: Option<&str>,
    all: bool,
) -> Result<()> {
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

    let skills = choose_skills(format, skill_name, all)?;
    if skills.is_empty() {
        if !format.is_json() {
            output::info("no skills selected — nothing installed");
        }
        return Ok(());
    }
    let outcomes = skill::install(&root, &agents, &skills, force)?;
    let rows: Vec<OutcomeRow> = outcomes
        .iter()
        .map(|o| OutcomeRow {
            path: o.path.display().to_string(),
            agent: o.agent.clone(),
            skill: o.skill.clone(),
            action: o.action.as_str().to_string(),
        })
        .collect();

    match format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => {
            for row in &rows {
                let line = format!("{} {}", row.action, row.path);
                // `skill::install()` (the only producer of these rows) never
                // emits `Pruned` or `Failed` — those come only from
                // `refresh_tracked`, reached through `bb update` and the
                // pre-command auto-refresh, not this command. No arm for them
                // here, so a future wiring mistake falls through to the
                // generic success line instead of silently matching nothing.
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
                    skill: r.skill.clone(),
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
                        r.skill.clone(),
                        r.agent.clone(),
                        r.state.as_str().to_string(),
                    ]
                })
                .collect();
            output::print_table(&["PATH", "SKILL", "AGENT", "STATE"], table_rows);
            output::info(&format!(
                "{} tracked skill{}",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            ));
        }
    }
    Ok(())
}

pub fn uninstall(
    format: Format,
    global: bool,
    force: bool,
    skill_name: Option<&str>,
) -> Result<()> {
    let root = if global {
        home_dir()?
    } else {
        std::env::current_dir().map_err(BbError::Io)?
    };

    let skills = wanted_skills(skill_name)?;
    let results = skill::uninstall(Some(&root), &skills, force)?;

    match format {
        Format::Json => {
            let json_rows: Vec<UninstallRowJson> = results
                .iter()
                .map(|(path, skill, outcome)| UninstallRowJson {
                    path: path.display().to_string(),
                    skill: skill.clone(),
                    outcome: outcome.as_str().to_string(),
                })
                .collect();
            output::print_json(&json_rows)?;
        }
        Format::Human => {
            if results.is_empty() {
                output::info("nothing to uninstall");
            }
            for (path, _skill, outcome) in &results {
                match outcome {
                    skill::RemovalOutcome::Removed => {
                        output::success(&format!("removed {}", path.display()))
                    }
                    skill::RemovalOutcome::RefusedModified => output::warn(&format!(
                        "{} was edited locally — left alone (pass --force to remove)",
                        path.display()
                    )),
                    skill::RemovalOutcome::RefusedUnsafePath => output::warn(&format!(
                        "{} does not look like a skill path bb would have written — left alone",
                        path.display()
                    )),
                    skill::RemovalOutcome::Absent => output::info(&format!(
                        "{} was already gone — nothing to remove",
                        path.display()
                    )),
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pick_skills_all_picked() {
        let wanted: Vec<&'static skill::Skill> = skill::SKILLS.iter().collect();
        let options: Vec<String> = wanted
            .iter()
            .map(|s| format!("{} — {}", s.name, s.summary))
            .collect();
        let picked = options.clone();

        let result = pick_skills(&wanted, &options, &picked);

        assert_eq!(result.len(), wanted.len());
    }

    #[test]
    fn pick_skills_some_picked() {
        let wanted: Vec<&'static skill::Skill> = skill::SKILLS.iter().collect();
        let options: Vec<String> = wanted
            .iter()
            .map(|s| format!("{} — {}", s.name, s.summary))
            .collect();
        assert!(
            options.len() >= 2,
            "test needs at least two skills to pick a subset"
        );
        let picked = vec![options[0].clone()];

        let result = pick_skills(&wanted, &options, &picked);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, wanted[0].name);
    }

    #[test]
    fn pick_skills_none_picked() {
        let wanted: Vec<&'static skill::Skill> = skill::SKILLS.iter().collect();
        let options: Vec<String> = wanted
            .iter()
            .map(|s| format!("{} — {}", s.name, s.summary))
            .collect();
        let picked: Vec<String> = Vec::new();

        let result = pick_skills(&wanted, &options, &picked);

        assert!(result.is_empty());
    }
}
