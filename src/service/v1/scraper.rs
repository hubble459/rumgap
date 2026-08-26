//! Admin-only per-hostname scraper health, backing an admin status page for keeping track of
//! which sites' scrapers are currently broken (see `util::scrape_log`, which writes the
//! `scrape_log` rows this reads).

use std::collections::HashMap;

use sea_orm::{DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};
use tonic::{Request, Response, Status};

use crate::interceptor::auth::UserPermissions;
use crate::proto::scraper_server::{Scraper, ScraperServer};
use crate::proto::{ScraperErrorEntry, ScraperStatus, ScraperStatusReply, ScraperStatusRequest};
use crate::util::db::DatabaseRequest;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

#[derive(Debug, FromQueryResult)]
struct LatestAttempt {
    hostname: String,
    last_attempt_at: chrono::NaiveDateTime,
    last_attempt_success: bool,
}

#[derive(Debug, FromQueryResult)]
struct HostnameAggregate {
    hostname: String,
    last_success_at: Option<chrono::NaiveDateTime>,
    attempts_24h: i64,
    failures_24h: i64,
}

#[derive(Debug, FromQueryResult)]
struct RecentError {
    hostname: String,
    operation: String,
    url: String,
    error_type: Option<String>,
    error_message: Option<String>,
    created_at: chrono::NaiveDateTime,
}

/// One row per hostname that has ever been scraped -- its single most recent attempt (and
/// whether that specific attempt succeeded).
async fn latest_attempts(db: &DatabaseConnection) -> Result<Vec<LatestAttempt>, Status> {
    LatestAttempt::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"
            SELECT DISTINCT ON (hostname)
                hostname,
                created_at AS last_attempt_at,
                success AS last_attempt_success
            FROM scrape_log
            ORDER BY hostname, created_at DESC
        "#,
    ))
    .all(db)
    .await
    .map_err(internal)
}

/// One row per hostname with its all-time last success and rolling 24h attempt/failure counts.
async fn hostname_aggregates(db: &DatabaseConnection) -> Result<Vec<HostnameAggregate>, Status> {
    HostnameAggregate::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"
            SELECT
                hostname,
                MAX(created_at) FILTER (WHERE success) AS last_success_at,
                COUNT(*) FILTER (WHERE created_at > now() - interval '24 hours') AS attempts_24h,
                COUNT(*) FILTER (WHERE NOT success AND created_at > now() - interval '24 hours') AS failures_24h
            FROM scrape_log
            GROUP BY hostname
        "#,
    ))
    .all(db)
    .await
    .map_err(internal)
}

/// Up to the 5 most recent failed attempts per hostname, newest first.
async fn recent_errors(db: &DatabaseConnection) -> Result<Vec<RecentError>, Status> {
    RecentError::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"
            SELECT hostname, operation, url, error_type, error_message, created_at
            FROM (
                SELECT
                    hostname, operation, url, error_type, error_message, created_at,
                    ROW_NUMBER() OVER (PARTITION BY hostname ORDER BY created_at DESC) AS rn
                FROM scrape_log
                WHERE NOT success
            ) ranked
            WHERE rn <= 5
            ORDER BY hostname, created_at DESC
        "#,
    ))
    .all(db)
    .await
    .map_err(internal)
}

#[derive(Debug, Default)]
pub struct ScraperController;

#[tonic::async_trait]
impl Scraper for ScraperController {
    /// Per-hostname scraper status: last attempt, last success, a rolling 24h health signal,
    /// and the most recent errors -- everything needed to spot a scraper that's quietly been
    /// broken for a while and go fix or replace it.
    async fn status(&self, request: Request<ScraperStatusRequest>) -> Result<Response<ScraperStatusReply>, Status> {
        let db = request.db()?;

        let latest = latest_attempts(db).await?;
        let mut aggregates: HashMap<String, HostnameAggregate> = hostname_aggregates(db)
            .await?
            .into_iter()
            .map(|a| (a.hostname.clone(), a))
            .collect();

        let mut errors_by_hostname: HashMap<String, Vec<ScraperErrorEntry>> = HashMap::new();
        for error in recent_errors(db).await? {
            let error_type = crate::proto::ScrapeErrorType::from_str_name(error.error_type.as_deref().unwrap_or(""))
                .unwrap_or_default()
                .into();

            errors_by_hostname
                .entry(error.hostname)
                .or_default()
                .push(ScraperErrorEntry {
                    operation: error.operation,
                    url: error.url,
                    error_type,
                    message: error.error_message.unwrap_or_default(),
                    created_at: error.created_at.and_utc().timestamp_millis(),
                });
        }

        let mut items: Vec<ScraperStatus> = latest
            .into_iter()
            .map(|attempt| {
                let aggregate = aggregates.remove(&attempt.hostname);
                ScraperStatus {
                    last_attempt_at: Some(attempt.last_attempt_at.and_utc().timestamp_millis()),
                    last_attempt_success: attempt.last_attempt_success,
                    last_success_at: aggregate
                        .as_ref()
                        .and_then(|a| a.last_success_at)
                        .map(|date| date.and_utc().timestamp_millis()),
                    attempts_24h: aggregate.as_ref().map_or(0, |a| a.attempts_24h),
                    failures_24h: aggregate.as_ref().map_or(0, |a| a.failures_24h),
                    recent_errors: errors_by_hostname.remove(&attempt.hostname).unwrap_or_default(),
                    hostname: attempt.hostname,
                }
            })
            .collect();

        items.sort_by(|a, b| a.hostname.cmp(&b.hostname));

        Ok(Response::new(ScraperStatusReply { items }))
    }
}

crate::export_service!(ScraperServer, ScraperController, auth = UserPermissions::ADMIN);
