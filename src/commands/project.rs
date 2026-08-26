use crate::api::models::Project;
use crate::error::Result;
use crate::output::{self, Format};
use crate::workspace::{self, WorkspaceCtx};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ProjectRow {
    key: String,
    name: String,
    access: String,
}

fn rows(projects: &[Project], name: Option<String>, limit: usize) -> Vec<ProjectRow> {
    let needle = name.map(|n| n.to_lowercase());
    projects
        .iter()
        .filter(|p| match &needle {
            Some(needle) => {
                p.name_or_dash().to_lowercase().contains(needle)
                    || p.key_or_dash().to_lowercase().contains(needle)
            }
            None => true,
        })
        .take(limit)
        .map(|p| ProjectRow {
            key: p.key_or_dash().to_string(),
            name: p.name_or_dash().to_string(),
            access: p.access().to_string(),
        })
        .collect()
}

pub async fn list(ctx: &WorkspaceCtx, name: Option<String>, limit: usize) -> Result<()> {
    let spinner = output::spinner("fetching projects");
    let projects = workspace::projects(ctx).await?;
    spinner.finish_and_clear();

    let rows = rows(&projects, name, limit);

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["KEY", "NAME", "ACCESS"],
            rows.iter()
                .map(|r| vec![r.key.clone(), r.name.clone(), r.access.clone()])
                .collect(),
        ),
    }
    Ok(())
}
