//! Comments on a manga, threaded to any depth through `parent_id`. Top-level comments list
//! newest first, replies oldest first (they read as a conversation). Deleting a comment that has
//! replies keeps it as "[deleted]" so the thread under it survives; one without replies goes away,
//! along with any "[deleted]" ancestors it was the last reply of.

use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Statement, TransactionTrait};
use tonic::{Request, Response, Status};

use crate::interceptor::auth::{UserHasPermissions, UserPermissions};
use crate::proto::comment_server::{Comment, CommentServer};
use crate::proto::{
    CommentReply, CommentThreadReply, CommentsReply, CreateCommentRequest, DeleteCommentRequest, EditCommentRequest,
    Empty, GetCommentRequest, ListCommentsRequest, PaginateReply,
};
use crate::util::auth::Authorize;
use crate::util::db::DatabaseRequest;

const MAX_BODY_CHARS: usize = 5000;

fn internal<E: ToString>(e: E) -> Status {
    Status::internal(e.to_string())
}

async fn query<C: ConnectionTrait>(db: &C, sql: &str, values: Vec<sea_orm::Value>) -> Result<Vec<QueryResult>, Status> {
    db.query_all_raw(Statement::from_sql_and_values(db.get_database_backend(), sql, values))
        .await
        .map_err(internal)
}

/// Columns of a `CommentReply` for comment `c`.
const COLUMNS: &str = r#"
    c.id, c.manga_id, c.parent_id, c.user_id, COALESCE(u.username, '') AS username,
    CASE WHEN c.deleted_at IS NULL THEN c.body ELSE '' END AS body,
    c.created_at, c.edited_at, c.deleted_at IS NOT NULL AS deleted,
    (SELECT COUNT(*) FROM comment r WHERE r.parent_id = c.id) AS reply_count
"#;
const FROM: &str = r#"FROM comment c LEFT JOIN "user" u ON u.id = c.user_id"#;

fn millis(at: chrono::NaiveDateTime) -> i64 {
    at.and_utc().timestamp_millis()
}

fn reply(row: &QueryResult, viewer: &entity::user::Model) -> Result<CommentReply, Status> {
    let get = || -> Result<CommentReply, sea_orm::DbErr> {
        let user_id: Option<i32> = row.try_get("", "user_id")?;
        let deleted: bool = row.try_get("", "deleted")?;
        let is_mine = user_id == Some(viewer.id);
        let edited_at: Option<chrono::NaiveDateTime> = row.try_get("", "edited_at")?;
        Ok(CommentReply {
            id: row.try_get("", "id")?,
            manga_id: row.try_get("", "manga_id")?,
            parent_id: row.try_get("", "parent_id")?,
            user_id: if deleted { None } else { user_id },
            username: if deleted {
                String::new()
            } else {
                row.try_get("", "username")?
            },
            body: row.try_get("", "body")?,
            created_at: millis(row.try_get("", "created_at")?),
            edited_at: edited_at.map(millis),
            deleted,
            reply_count: row.try_get("", "reply_count")?,
            is_mine: is_mine && !deleted,
            can_delete: !deleted && (is_mine || viewer.has_permission(UserPermissions::ADMIN)),
        })
    };
    get().map_err(internal)
}

async fn get_comment<C: ConnectionTrait>(
    db: &C,
    id: i32,
    viewer: &entity::user::Model,
) -> Result<CommentReply, Status> {
    let rows = query(db, &format!("SELECT {COLUMNS} {FROM} WHERE c.id = $1"), vec![id.into()]).await?;
    reply(rows.first().ok_or(Status::not_found("Comment not found"))?, viewer)
}

fn clean_body(body: &str) -> Result<String, Status> {
    let body = body.trim();
    if body.is_empty() {
        return Err(Status::invalid_argument("A comment can't be empty"));
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(Status::invalid_argument(format!(
            "A comment can be at most {MAX_BODY_CHARS} characters"
        )));
    }
    Ok(body.to_string())
}

#[derive(Debug, Default)]
pub struct CommentController;

#[tonic::async_trait]
impl Comment for CommentController {
    async fn list(&self, request: Request<ListCommentsRequest>) -> Result<Response<CommentsReply>, Status> {
        let db = request.db()?;
        let viewer = request.authorize()?;
        let req = request.get_ref();
        let per_page = req.per_page.unwrap_or(20).clamp(1, 100);

        let (filter, order, values): (&str, &str, Vec<sea_orm::Value>) = match req.parent_id {
            Some(parent_id) => ("c.parent_id = $1", "c.created_at, c.id", vec![parent_id.into()]),
            None => (
                "c.manga_id = $1 AND c.parent_id IS NULL",
                "c.created_at DESC, c.id DESC",
                vec![req.manga_id.into()],
            ),
        };

        let total: i64 = query(
            db,
            &format!("SELECT COUNT(*) AS total FROM comment c WHERE {filter}"),
            values.clone(),
        )
        .await?
        .first()
        .map(|row| row.try_get("", "total"))
        .transpose()
        .map_err(internal)?
        .unwrap_or_default();
        let total = total as u64;
        let max_page = total.div_ceil(per_page).saturating_sub(1);
        let page = req.page.unwrap_or(0).min(max_page);

        let mut paged = values;
        paged.push((per_page as i64).into());
        paged.push(((page * per_page) as i64).into());
        let rows = query(
            db,
            &format!("SELECT {COLUMNS} {FROM} WHERE {filter} ORDER BY {order} LIMIT $2 OFFSET $3"),
            paged,
        )
        .await?;

        let total_comments: i64 = query(
            db,
            "SELECT COUNT(*) AS total FROM comment WHERE manga_id = $1 AND deleted_at IS NULL",
            vec![req.manga_id.into()],
        )
        .await?
        .first()
        .map(|row| row.try_get("", "total"))
        .transpose()
        .map_err(internal)?
        .unwrap_or_default();

        Ok(Response::new(CommentsReply {
            pagination: Some(PaginateReply {
                total,
                page,
                per_page,
                max_page,
            }),
            items: rows.iter().map(|row| reply(row, viewer)).collect::<Result<_, _>>()?,
            total_comments,
        }))
    }

    async fn thread(&self, request: Request<GetCommentRequest>) -> Result<Response<CommentThreadReply>, Status> {
        let db = request.db()?;
        let viewer = request.authorize()?;
        let id = request.get_ref().id;

        let comment = get_comment(db, id, viewer).await?;
        let rows = query(
            db,
            &format!(
                r#"
                WITH RECURSIVE up AS (
                    SELECT parent_id AS id, 1 AS depth FROM comment WHERE id = $1
                    UNION ALL
                    SELECT p.parent_id, up.depth + 1 FROM comment p JOIN up ON p.id = up.id
                    WHERE up.id IS NOT NULL
                )
                SELECT {COLUMNS} {FROM} JOIN up ON up.id = c.id ORDER BY up.depth DESC
                "#
            ),
            vec![id.into()],
        )
        .await?;

        Ok(Response::new(CommentThreadReply {
            comment: Some(comment),
            ancestors: rows.iter().map(|row| reply(row, viewer)).collect::<Result<_, _>>()?,
        }))
    }

    async fn create(&self, request: Request<CreateCommentRequest>) -> Result<Response<CommentReply>, Status> {
        let db = request.db()?;
        let viewer = request.authorize()?;
        let req = request.get_ref();
        let body = clean_body(&req.body)?;

        if let Some(parent_id) = req.parent_id {
            let parent = query(
                db,
                "SELECT manga_id, deleted_at IS NOT NULL AS deleted FROM comment WHERE id = $1",
                vec![parent_id.into()],
            )
            .await?;
            let parent = parent
                .first()
                .ok_or(Status::not_found("The comment you're replying to is gone"))?;
            let manga_id: i32 = parent.try_get("", "manga_id").map_err(internal)?;
            let deleted: bool = parent.try_get("", "deleted").map_err(internal)?;
            if manga_id != req.manga_id {
                return Err(Status::invalid_argument("That reply belongs to a different manga"));
            }
            if deleted {
                return Err(Status::failed_precondition("You can't reply to a deleted comment"));
            }
        }

        let rows = query(
            db,
            "INSERT INTO comment (manga_id, user_id, parent_id, body) VALUES ($1, $2, $3, $4) RETURNING id",
            vec![req.manga_id.into(), viewer.id.into(), req.parent_id.into(), body.into()],
        )
        .await
        .map_err(|status| {
            if status.message().contains("comment_manga_id_fkey") {
                Status::not_found("Manga not found")
            } else {
                status
            }
        })?;
        let id: i32 = rows
            .first()
            .ok_or(Status::internal("Comment wasn't saved"))?
            .try_get("", "id")
            .map_err(internal)?;

        Ok(Response::new(get_comment(db, id, viewer).await?))
    }

    async fn edit(&self, request: Request<EditCommentRequest>) -> Result<Response<CommentReply>, Status> {
        let db = request.db()?;
        let viewer = request.authorize()?;
        let req = request.get_ref();
        let body = clean_body(&req.body)?;

        let existing = get_comment(db, req.id, viewer).await?;
        if !existing.is_mine {
            return Err(Status::permission_denied("You can only edit your own comments"));
        }

        query(
            db,
            "UPDATE comment SET body = $2, edited_at = NOW() WHERE id = $1",
            vec![req.id.into(), body.into()],
        )
        .await?;

        Ok(Response::new(get_comment(db, req.id, viewer).await?))
    }

    async fn delete(&self, request: Request<DeleteCommentRequest>) -> Result<Response<Empty>, Status> {
        let db: &DatabaseConnection = request.db()?;
        let viewer = request.authorize()?;
        let id = request.get_ref().id;

        let existing = get_comment(db, id, viewer).await?;
        if !existing.can_delete {
            return Err(Status::permission_denied("You can only delete your own comments"));
        }

        let txn = db.begin().await.map_err(internal)?;
        if existing.reply_count > 0 {
            query(
                &txn,
                "UPDATE comment SET deleted_at = NOW(), body = '' WHERE id = $1",
                vec![id.into()],
            )
            .await?;
        } else {
            // Gone entirely, and so is every "[deleted]" ancestor this was the last reply of.
            let mut current = Some(id);
            while let Some(comment_id) = current {
                let rows = query(
                    &txn,
                    "DELETE FROM comment WHERE id = $1 RETURNING parent_id",
                    vec![comment_id.into()],
                )
                .await?;
                let parent_id: Option<i32> = match rows.first() {
                    Some(row) => row.try_get("", "parent_id").map_err(internal)?,
                    None => None,
                };
                current = match parent_id {
                    Some(parent_id) => {
                        let orphaned = query(
                            &txn,
                            r#"
                            SELECT 1 FROM comment p
                            WHERE p.id = $1 AND p.deleted_at IS NOT NULL
                              AND NOT EXISTS (SELECT 1 FROM comment r WHERE r.parent_id = p.id)
                            "#,
                            vec![parent_id.into()],
                        )
                        .await?;
                        (!orphaned.is_empty()).then_some(parent_id)
                    }
                    None => None,
                };
            }
        }
        txn.commit().await.map_err(internal)?;

        Ok(Response::new(Empty {}))
    }
}

crate::export_service!(CommentServer, CommentController, auth = UserPermissions::USER);
