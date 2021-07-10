use rocket::{Rocket, Build, futures};
use rocket::fairing::{self, AdHoc};
use rocket::response::status::Created;
use rocket::serde::{Serialize, Deserialize, json::Json};

use rocket_db_pools::{sqlx, Database};

use futures::stream::TryStreamExt;
use futures::future::TryFutureExt;

#[derive(Database)]
#[database("sqlx")]
struct Db(sqlx::SqlitePool);

type Connection = rocket_db_pools::Connection<Db>;

type Result<T, E = rocket::response::Debug<sqlx::Error>> = std::result::Result<T, E>;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(crate = "rocket::serde")]
struct Post {
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    title: String,
    text: String,
}

#[post("/", data = "<post>")]
async fn create(mut db: Connection, post: Json<Post>) -> Result<Created<Json<Post>>> {
    // There is no support for `RETURNING`.
    sqlx::query!("INSERT INTO posts (title, text) VALUES (?, ?)", post.title, post.text)
        .execute(&mut *db)
        .await?;

    Ok(Created::new("/").body(post))
}

#[get("/")]
async fn list(mut db: Connection) -> Result<Json<Vec<i64>>> {
    let ids = sqlx::query!("SELECT id FROM posts")
        .fetch(&mut *db)
        .map_ok(|record| record.id)
        .try_collect::<Vec<_>>()
        .await?;

    Ok(Json(ids))
}

#[get("/<id>")]
async fn read(mut db: Connection, id: i64) -> Option<Json<Post>> {
    sqlx::query!("SELECT id, title, text FROM posts WHERE id = ?", id)
        .fetch_one(&mut *db)
        .map_ok(|r| Json(Post { id: Some(r.id), title: r.title, text: r.text }))
        .await
        .ok()
}

#[delete("/<id>")]
async fn delete(mut db: Connection, id: i64) -> Result<Option<()>> {
    let result = sqlx::query!("DELETE FROM posts WHERE id = ?", id)
        .execute(&mut *db)
        .await?;

    Ok((result.rows_affected() == 1).then(|| ()))
}

#[delete("/")]
async fn destroy(mut db: Connection) -> Result<()> {
    sqlx::query!("DELETE FROM posts").execute(&mut *db).await?;

    Ok(())
}

async fn init_db(rocket: Rocket<Build>) -> fairing::Result {
    match rocket.state::<Db>() {
        Some(db) => {
            if let Err(e) = sqlx::migrate!("db/sqlx/migrations").run(db.pool()).await {
                error!("Failed to initialize SQLx database: {}", e);
                return Err(rocket);
            }
            Ok(rocket)
        }
        None => Err(rocket),
    }
}

pub fn stage() -> AdHoc {
    AdHoc::on_ignite("SQLx Stage", |rocket| async {
        rocket
            .attach(Db::fairing())
            .attach(AdHoc::try_on_ignite("SQLx Database", init_db))
            .mount("/sqlx", routes![list, create, read, delete, destroy])
    })
}
