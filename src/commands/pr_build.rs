use crate::api::models::BuildStatus;
use crate::commands::pr::Ctx;
use crate::error::Result;
use futures::stream::{self, StreamExt, TryStreamExt};
use std::collections::HashMap;

/// Bitbucket exposes build status only per pull request, so a column over N rows
/// costs N requests. Cap how many are in flight: enough to be fast on a busy
/// repository, few enough to stay clear of the rate limit.
const MAX_IN_FLIGHT: usize = 8;

pub async fn statuses(ctx: &Ctx, id: u64) -> Result<Vec<BuildStatus>> {
    ctx.client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/statuses")))
        .await
}

pub async fn statuses_for(ctx: &Ctx, ids: &[u64]) -> Result<HashMap<u64, Vec<BuildStatus>>> {
    stream::iter(ids.iter().copied())
        .map(|id| async move { statuses(ctx, id).await.map(|s| (id, s)) })
        .buffer_unordered(MAX_IN_FLIGHT)
        .try_collect()
        .await
}
