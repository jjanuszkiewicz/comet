use crate::api::gog::stats::{FieldValue, Stat};
use crate::api::handlers::context::HandlerContext;
use crate::paths;
use log::info;
use sqlx::{Acquire, Error, Executor, Row, SqlitePool};

pub const SETUP_QUERY: &str = r#"
CREATE TABLE IF NOT EXISTS `leaderboard` (`id` INTEGER PRIMARY KEY NOT NULL,`key` TEXT UNIQUE NOT NULL,`name` TEXT NOT NULL,`sort_method` TEXT CHECK ( sort_method IN ( 'SORT_METHOD_ASCENDING', 'SORT_METHOD_DESCENDING' ) ) NOT NULL,`display_type` TEXT CHECK ( display_type IN ( 'DISPLAY_TYPE_NUMERIC', 'DISPLAY_TYPE_TIME_SECONDS', 'DISPLAY_TYPE_TIME_MILLISECONDS' ) ) NOT NULL,`score` INTEGER NOT NULL DEFAULT 0,`rank` INTEGER NOT NULL DEFAULT 0,`force_update` INTEGER CHECK ( force_update IN ( 0, 1 ) ) NOT NULL DEFAULT 0,`changed` INTEGER CHECK ( changed IN ( 0, 1 ) ) NOT NULL, entry_total_count INTEGER NOT NULL DEFAULT 0, details TEXT NOT NULL DEFAULT "");
CREATE TABLE IF NOT EXISTS `achievement` (`id` INTEGER PRIMARY KEY NOT NULL,`key` TEXT UNIQUE NOT NULL,`name` TEXT NOT NULL,`description` TEXT NOT NULL,`visible_while_locked` INTEGER CHECK ( visible_while_locked IN ( 0, 1 ) ) NOT NULL,`unlock_time` TEXT,`image_url_locked` TEXT NOT NULL,`image_url_unlocked` TEXT NOT NULL,`changed` INTEGER CHECK ( changed IN ( 0, 1 ) ) NOT NULL, rarity REAL NOT NULL DEFAULT 0.0, rarity_level_description TEXT NOT NULL DEFAULT "", rarity_level_slug TEXT NOT NULL DEFAULT "");
CREATE TABLE IF NOT EXISTS `statistic` (`id` INTEGER PRIMARY KEY NOT NULL,`key` TEXT UNIQUE NOT NULL,`type` TEXT CHECK ( type IN ( 'INT', 'FLOAT', 'AVGRATE' ) ) NOT NULL,`increment_only` INTEGER CHECK ( increment_only IN ( 0, 1 ) ) NOT NULL,`changed` INTEGER CHECK ( changed IN ( 0, 1 ) ) NOT NULL);
CREATE INDEX IF NOT EXISTS `is_leaderboard_score_changed` on leaderboard (changed);
CREATE INDEX IF NOT EXISTS `is_achievement_changed` ON achievement (changed);
CREATE INDEX IF NOT EXISTS `is_statistic_changed` ON statistic (changed);
CREATE TABLE IF NOT EXISTS `game_info` (`time_played` INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS `int_statistic` (`id` INTEGER REFERENCES statistic ( id ) NOT NULL,`value` INTEGER NOT NULL DEFAULT 0,`default_value` INTEGER NOT NULL DEFAULT 0,`min_value` INTEGER,`max_value` INTEGER,`max_change` INTEGER);
CREATE TABLE IF NOT EXISTS `float_statistic` (`id` INTEGER REFERENCES statistic ( id ) NOT NULL,`value` REAL NOT NULL DEFAULT 0,`default_value` REAL NOT NULL DEFAULT 0,`min_value` REAL,`max_value` REAL,`max_change` REAL,`window` REAL DEFAULT NULL);
CREATE TABLE IF NOT EXISTS `database_info` (`key` TEXT PRIMARY KEY NOT NULL,`value` TEXT NOT NULL);
"#;

pub async fn setup_connection(client_id: &str, user_id: &str) -> Result<SqlitePool, Error> {
    let databases_path = paths::GAMEPLAY_STORAGE.join(client_id).join(user_id);
    let database_file = databases_path.join("gameplay.db");
    if !databases_path.exists() {
        let _ = tokio::fs::create_dir_all(&databases_path).await;
    }

    if !database_file.exists() {
        let _ = tokio::fs::File::create(&database_file).await;
    }

    info!("Setting up database at {:?}", database_file);
    let url = String::from("sqlite:") + database_file.to_str().unwrap();

    SqlitePool::connect(&url).await
}

pub async fn has_statistics(context: &mut HandlerContext) -> bool {
    let database = context.db_connection();
    let mut connection = database.acquire().await;
    if let Err(_) = connection {
        return false;
    }
    let mut connection = connection.unwrap();
    let res = sqlx::query("SELECT * FROM database_info WHERE key='stats_retrieved'")
        .fetch_one(&mut *connection)
        .await;

    match res {
        Ok(result) => {
            let value = result
                .try_get("value")
                .unwrap_or("0")
                .parse::<u8>()
                .unwrap();
            !result.is_empty() && value != 0
        }
        Err(_) => false,
    }
}

pub async fn get_statistics(context: &mut HandlerContext) -> Result<Vec<Stat>, Error> {
    let database = context.db_connection();
    let mut connection = database.acquire().await?;
    let mut stats: Vec<Stat> = Vec::new();
    let int_stats = sqlx::query(
        r#"SELECT s.id, s.key, s.increment_only,
        i.value, i.default_value, i.min_value, i.max_value, i.max_change
        FROM int_statistic AS i
        JOIN statistic AS s
        ON s.id = i.id"#,
    )
    .fetch_all(&mut *connection)
    .await?;
    let float_stats = sqlx::query(
        r#"SELECT s.id, s.key, s.type, s.increment_only,
        f.value, f.default_value, f.min_value, f.max_value, f.max_change, f.window
        FROM float_statistic AS f
        JOIN statistic AS s
        ON s.id = f.id"#,
    )
    .fetch_all(&mut *connection)
    .await?;

    for int_stat in int_stats {
        let id: i64 = int_stat.try_get("id").unwrap();
        let key: String = int_stat.try_get("key").unwrap();
        let increment_only: u8 = int_stat.try_get("increment_only").unwrap();
        let values = FieldValue::INT {
            value: int_stat.try_get("value").unwrap(),
            default_value: int_stat.try_get("default_value").unwrap(),
            min_value: int_stat.try_get("min_value").unwrap(),
            max_value: int_stat.try_get("max_value").unwrap(),
            max_change: int_stat.try_get("max_change").unwrap(),
        };
        let new_stat = Stat::new(id.to_string(), key, None, increment_only == 1, values);
        stats.push(new_stat)
    }

    for float_stat in float_stats {
        let id: i64 = float_stat.try_get("id").unwrap();
        let key: String = float_stat.try_get("key").unwrap();
        let increment_only: u8 = float_stat.try_get("increment_only").unwrap();
        let window: Option<f64> = float_stat.try_get("window").unwrap();
        let value_type: String = float_stat.try_get("type").unwrap();
        let values: FieldValue = match value_type.as_str() {
            "FLOAT" => FieldValue::FLOAT {
                value: float_stat.try_get("value").unwrap(),
                default_value: float_stat.try_get("default_value").unwrap(),
                min_value: float_stat.try_get("min_value").unwrap(),
                max_value: float_stat.try_get("max_value").unwrap(),
                max_change: float_stat.try_get("max_change").unwrap(),
            },
            "AVGRATE" => FieldValue::AVGRATE {
                value: float_stat.try_get("value").unwrap(),
                default_value: float_stat.try_get("default_value").unwrap(),
                min_value: float_stat.try_get("min_value").unwrap(),
                max_value: float_stat.try_get("max_value").unwrap(),
                max_change: float_stat.try_get("max_change").unwrap(),
            },
            _ => panic!("Unsupported value type"),
        };
        let new_stat = Stat::new(id.to_string(), key, window, increment_only == 1, values);
        stats.push(new_stat)
    }

    Ok(stats)
}

pub async fn set_statistics(context: &mut HandlerContext, stats: &Vec<Stat>) -> Result<(), Error> {
    let database = context.db_connection();
    let mut connection = database.acquire().await?;
    let mut transaction = connection.begin().await?;

    sqlx::query("DELETE FROM int_statistic; DELETE FROM float_statistic; DELETE FROM statistic;")
        .execute(&mut *transaction)
        .await?;

    for stat in stats {
        let stat_id = stat.stat_id().parse::<i64>().unwrap();
        let stat_type = match stat.values() {
            FieldValue::INT { .. } => "INT",
            FieldValue::FLOAT { .. } => "FLOAT",
            FieldValue::AVGRATE { .. } => "AVGRATE",
        };
        sqlx::query("INSERT INTO statistic VALUES ($1, $2, $3, $4, 0)")
            .bind(stat_id)
            .bind(stat.stat_key())
            .bind(stat_type)
            .bind(stat.increment_only().to_owned() as u8)
            .execute(&mut *transaction)
            .await?;

        match stat.values() {
            FieldValue::INT {
                value,
                default_value,
                max_value,
                min_value,
                max_change,
            } => {
                sqlx::query("INSERT INTO int_statistic VALUES ($1, $2, $3, $4, $5, $6)")
                    .bind(stat_id)
                    .bind(value)
                    .bind(default_value.unwrap_or_else(|| 0))
                    .bind(min_value)
                    .bind(max_value)
                    .bind(max_change)
                    .execute(&mut *transaction)
                    .await?;
            }

            FieldValue::FLOAT {
                value,
                default_value,
                min_value,
                max_value,
                max_change,
            }
            | FieldValue::AVGRATE {
                value,
                default_value,
                min_value,
                max_value,
                max_change,
            } => {
                sqlx::query("INSERT INTO float_statistic VALUES ($1, $2, $3, $4, $5, $6, $7)")
                    .bind(stat_id)
                    .bind(value)
                    .bind(default_value.unwrap_or_else(|| 0.0))
                    .bind(min_value)
                    .bind(max_value)
                    .bind(max_change)
                    .bind(stat.window())
                    .execute(&mut *transaction)
                    .await?;
            }
        }
    }

    let _ = sqlx::query("INSERT INTO database_info VALUES ('stats_retrieved', '1')")
        .execute(&mut *transaction)
        .await;

    let _ = sqlx::query("UPDATE database_info SET value='1' WHERE key='stats_retrieved'")
        .execute(&mut *transaction)
        .await;

    transaction.commit().await?;
    Ok(())
}
