use rocket::{Rocket, State, Build, futures};
use rocket::fairing::{self, AdHoc};
use rocket::response::status::Created;
use rocket::trace::error;
use rocket_contrib::json::Json;

use futures::stream::TryStreamExt;
use futures::future::TryFutureExt;
use serde::{Serialize, Deserialize};
use sqlx::ConnectOptions;

type Db = sqlx::SqlitePool;

type Result<T, E = rocket::response::Debug<sqlx::Error>> = std::result::Result<T, E>;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Post {
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    title: String,
    text: String,
}

#[post("/", data = "<post>")]
async fn create(db: State<'_, Db>, post: Json<Post>) -> Result<Created<Json<Post>>> {
    // There is no support for `RETURNING`.
    sqlx::query!("INSERT INTO posts (title, text) VALUES (?, ?)", post.title, post.text)
        .execute(&*db)
        .await?;

    Ok(Created::new("/").body(post))
}

#[get("/")]
async fn list(db: State<'_, Db>) -> Result<Json<Vec<i64>>> {
    let ids = sqlx::query!("SELECT id FROM posts")
        .fetch(&*db)
        .map_ok(|record| record.id)
        .try_collect::<Vec<_>>()
        .await?;

    Ok(Json(ids))
}

#[get("/<id>")]
async fn read(db: State<'_, Db>, id: i64) -> Option<Json<Post>> {
    sqlx::query!("SELECT id, title, text FROM posts WHERE id = ?", id)
        .fetch_one(&*db)
        .map_ok(|r| Json(Post { id: Some(r.id), title: r.title, text: r.text }))
        .await
        .ok()
}

#[delete("/<id>")]
async fn delete(db: State<'_, Db>, id: i64) -> Result<Option<()>> {
    let result = sqlx::query!("DELETE FROM posts WHERE id = ?", id)
        .execute(&*db)
        .await?;

    Ok((result.rows_affected() == 1).then(|| ()))
}

#[delete("/")]
async fn destroy(db: State<'_, Db>) -> Result<()> {
    sqlx::query!("DELETE FROM posts").execute(&*db).await?;

    Ok(())
}

async fn init_db(rocket: Rocket<Build>) -> fairing::Result {
    use rocket_contrib::databases::Config;

    let config = match Config::from("sqlx", &rocket) {
        Ok(config) => config,
        Err(e) => {
            error!("Failed to read SQLx config: {}", e);
            return Err(rocket);
        }
    };

    let mut opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&config.url)
        .create_if_missing(true);

    opts.disable_statement_logging();
    let db = match Db::connect_with(opts).await {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to SQLx database: {}", e);
            return Err(rocket);
        }
    };

    if let Err(e) = sqlx::migrate!("db/sqlx/migrations").run(&db).await {
        error!("Failed to initialize SQLx database: {}", e);
        return Err(rocket);
    }

    Ok(rocket.manage(db))
}

pub fn stage() -> AdHoc {
    AdHoc::on_ignite("SQLx Stage", |rocket| async {
        rocket
            .attach(AdHoc::try_on_ignite("SQLx Database", init_db))
            .mount("/sqlx", routes![list, create, read, delete, destroy])
    })
}
