use anyhow::Result;
use nexus_balancer::config::AppConfig;
use nexus_balancer::db::Database;
use nexus_balancer::run_server;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let config = AppConfig::load("config.yaml")?;
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:nexus.db".to_string());
    let db = Database::new(&db_url).await?;

    // ANSI Colors
    let cyan = "\x1b[36m";
    let green = "\x1b[32m";
    let yellow = "\x1b[33m";
    let blue = "\x1b[34m";
    let reset = "\x1b[0m";
    let bold = "\x1b[1m";

    println!(r#"{}{}
    _   _                       ____        _
   | \ | | _____  ___   _ ___  | __ )  __ _| | __ _ _ __   ___ ___ _ __
   |  \| |/ _ \ \/ / | | / __| |  _ \ / _` | |/ _` | '_ \ / __/ _ \ '__|
   | |\  |  __/>  <| |_| \__ \ | |_) | (_| | | (_| | | | | (_|  __/ |
   |_| \_|\___/_/\_\\__,_|___/ |____/ \__,_|_|\__,_|_| |_|\___\___|_|
    {}"#, bold, cyan, reset);

    println!("   {}----------------------------------------------------------------------{}", blue, reset);
    println!("    Status:  {}Running{}", green, reset);
    println!("    Address: {}http://{}:{}{}", bold, config.server.host, config.server.port, reset);
    let db_name = db_url.split(':').last().unwrap_or("nexus.db");
    println!("    Storage: SQLite ({})", db_name);
    println!("    MCP:     {}Enabled{}", yellow, reset);
    println!("    Docs:    {}http://{}:{}/scalar{}", bold, config.server.host, config.server.port, reset);
    println!("   {}----------------------------------------------------------------------{}", blue, reset);

    run_server(config, db, "secrets").await
}