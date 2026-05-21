use std::env;

use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

pub async fn connect() -> surrealdb::Result<Surreal<Client>> {
    let url = env::var("SURREAL_URL").unwrap_or_else(|_| "127.0.0.1:8000".into());
    let user = env::var("SURREAL_USER").unwrap_or_else(|_| "root".into());
    let pass = env::var("SURREAL_PASS").unwrap_or_else(|_| "root".into());
    let ns = env::var("SURREAL_NS").unwrap_or_else(|_| "authlyn".into());
    let db_name = env::var("SURREAL_DB").unwrap_or_else(|_| "dev".into());

    let host = url.trim_start_matches("ws://").trim_start_matches("wss://");
    let db = Surreal::new::<Ws>(host).await?;
    db.signin(Root {
        username: &user,
        password: &pass,
    })
    .await?;
    db.use_ns(ns).use_db(db_name).await?;
    Ok(db)
}
