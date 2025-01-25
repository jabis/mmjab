use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Arg, Command};
use dotenv::dotenv;
use log::{error, info, trace, warn};
use std::path::Path;
use std::{env, fs};
use tokio_postgres::{Client, NoTls};

#[tokio::main]
async fn main() {
  if env::var("RUST_LOG").is_err() {
    env::set_var("RUST_LOG", "info")
  }
  env_logger::init();
  dotenv().ok();
  let matches = Command::new("Clean Utility")
    .version("1.0")
    .about("Cleans old files and database rows based on retention policies")
    .arg(
      Arg::new("data_dir")
        .long("data-dir")
        .env("MATTERMOST_DATA_DIRECTORY")
        .help("Path to the Mattermost data directory")
        .required(true),
    )
    .arg(
      Arg::new("db_name")
        .short('n')
        .long("db-name")
        .env("PGDATABASE")
        .help("Database name")
        .required(true),
    )
    .arg(
      Arg::new("db_user")
        .short('u')
        .long("db-user")
        .env("PGUSER")
        .help("Database user")
        .required(true),
    )
    .arg(
      Arg::new("db_password")
        .short('p')
        .long("db-password")
        .env("PGPASSWORD")
        .help("Database password")
        .required(true),
    )
    .arg(
      Arg::new("db_host")
        .short('h')
        .long("db-host")
        .env("PGHOST")
        .help("Database host")
        .required(true),
    )
    .arg(
      Arg::new("db_port")
        .short('P')
        .long("db-port")
        .env("PGPORT")
        .help("Database port")
        .required(true),
    )
    .arg(
      Arg::new("retention_days")
        .short('D')
        .long("retention-days")
        .env("RETENTION_DAYS")
        .help("Number of days to retain data")
        .required(true),
    )
    .arg(
      Arg::new("file_batch_size")
        .short('b')
        .long("file-batch-size")
        .env("FILE_BATCH_SIZE")
        .help("Batch size for file deletion")
        .required(true),
    )
    .arg(
      Arg::new("remove_posts")
        .long("remove-posts")
        .help("Wipe posts older than timestamp")
        .required(false),
    )
    .arg(
      Arg::new("dry_run")
        .long("dry-run")
        .help("Perform a dry run without making any changes")
        .required(false),
    )
    .get_matches();

  let mattermost_data_directory = matches.get_one::<String>("data_dir").unwrap();
  let database_name = matches.get_one::<String>("db_name").unwrap();
  let database_user = matches.get_one::<String>("db_user").unwrap();
  let database_password = matches.get_one::<String>("db_password").unwrap();
  let database_host = matches.get_one::<String>("db_host").unwrap();
  let database_port = matches.get_one::<String>("db_port").unwrap();
  let retention_days = matches.get_one::<String>("retention_days").unwrap();
  let file_batch_size = matches.get_one::<String>("file_batch_size").unwrap();
  let remove_posts = matches.contains_id("remove_posts");
  let dry_run = matches.contains_id("dry_run");

  let retention_days = retention_days
    .parse::<i64>()
    .expect("fucking hell retention");
  let file_batch_size = file_batch_size
    .parse::<usize>()
    .expect("fucking hell batch size");

  if let Err(err) = clean(
    mattermost_data_directory,
    database_name,
    database_user,
    database_password,
    database_host,
    database_port,
    retention_days,
    file_batch_size,
    remove_posts,
    dry_run,
  )
  .await
  {
    error!("Cleaning operation failed: {}", err);
  } else {
    info!("Cleaning operation completed successfully.");
  }
}

#[allow(clippy::too_many_arguments)]
pub async fn clean(
  mattermost_data_directory: &str,
  database_name: &str,
  database_user: &str,
  database_password: &str,
  database_host: &str,
  database_port: &str,
  retention_days: i64,
  file_batch_size: usize,
  remove_posts: bool,
  dry_run: bool,
) -> Result<()> {
  validate(
    mattermost_data_directory,
    database_name,
    database_user,
    database_host,
    retention_days,
    file_batch_size,
  )?;

  let connection_string = format!(
    "postgres://{}:{}@{}:{}/{}?sslmode=disable",
    database_user, database_password, database_host, database_port, database_name
  );
  trace!("Connection string: {}", &connection_string);
  let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
    .await
    .context("Failed to connect to the database")?;

  tokio::spawn(async move {
    if let Err(e) = connection.await {
      warn!("error happened at spawn {e}");
      eprintln!("connection error: {}", e);
    }
  });
  info!("Connection established: OK");
  let millisecond_epoch = (Utc::now() - chrono::Duration::days(retention_days)).timestamp_millis();

  clean_files(
    &client,
    millisecond_epoch,
    mattermost_data_directory,
    file_batch_size,
    dry_run,
  )
  .await?;
  delete_file_info_rows(&client, millisecond_epoch, dry_run).await?;
  if remove_posts {
    delete_post_rows(&client, millisecond_epoch, dry_run).await?;
  } else {
    info!("Skipping posts removal")
  }

  Ok(())
}

async fn clean_files(
  client: &Client,
  millisecond_epoch: i64,
  mattermost_data_directory: &str,
  file_batch_size: usize,
  dry_run: bool,
) -> Result<()> {
  let mut batch = 0;
  let mut more_results = true;

  while more_results {
    more_results = clean_files_batch(
      client,
      millisecond_epoch,
      mattermost_data_directory,
      file_batch_size,
      batch,
      dry_run,
    )
    .await?;
    batch += 1;
  }

  Ok(())
}

async fn clean_files_batch(
  client: &Client,
  millisecond_epoch: i64,
  mattermost_data_directory: &str,
  file_batch_size: usize,
  batch: usize,
  dry_run: bool,
) -> Result<bool> {
  let query = "
        SELECT path, thumbnailpath, previewpath
        FROM fileinfo
        WHERE createat < $1
        OFFSET $2
        LIMIT $3;
    ";
  trace!("Querying: {}", &query);
  let offset = (batch * file_batch_size) as i64;
  let limit = file_batch_size as i64;
  trace!("params: {} {} {}", &millisecond_epoch, &offset, &limit);
  let rows = client
    .query(query, &[&millisecond_epoch, &offset, &limit])
    .await
    .context("Failed to fetch fileinfo rows")?;

  let mut more_results = false;

  for row in rows {
    more_results = true;
    let path: String = row.get("path");
    let thumbnail_path: String = row.get("thumbnailpath");
    let preview_path: String = row.get("previewpath");

    if dry_run {
      info!(
        "[DRY RUN] Would remove: {:?}, {:?}, {:?}",
        path, thumbnail_path, preview_path
      );
    } else {
      remove_files(
        mattermost_data_directory,
        &path,
        &thumbnail_path,
        &preview_path,
      )
      .context("Failed to remove files")?;
    }
  }

  Ok(more_results)
}

fn remove_files(
  base_dir: &str,
  path: &str,
  thumbnail_path: &str,
  preview_path: &str,
) -> Result<()> {
  let files = [path, thumbnail_path, preview_path];
  let mut num_deleted = 0;
  for file in files {
    if !file.is_empty() {
      let full_path = Path::new(base_dir).join(file);
      if full_path.exists() {
        fs::remove_file(full_path.clone())
          .context(format!("Failed to delete file: {:?}", &full_path))?;
        trace!("Removed: {:#?} ", &full_path);
        num_deleted += 1;
      } else {
        trace!("Path does not exist: {:#?} ", &full_path);
      }
    }
  }
  if num_deleted > 0 {
    info!("Deleted: {} files. Main file: {}", num_deleted, path);
  } else {
    trace!("No files to be deleted");
  }
  Ok(())
}

async fn delete_file_info_rows(
  client: &Client,
  millisecond_epoch: i64,
  dry_run: bool,
) -> Result<()> {
  let query = "
        DELETE FROM fileinfo
        WHERE createat < $1;
    ";
  trace!("Querying: {}", &query);
  trace!("Params: {:#?}", &millisecond_epoch);
  if dry_run {
    info!(
      "[DRY RUN] Would delete `fileinfo` rows older than {}",
      millisecond_epoch
    );
    return Ok(());
  }
  let result = client
    .execute(query, &[&millisecond_epoch])
    .await
    .context("Failed to delete `fileinfo` rows")?;
  info!("Removed {} fileinfo rows", result);
  Ok(())
}

async fn delete_post_rows(client: &Client, millisecond_epoch: i64, dry_run: bool) -> Result<()> {
  let query = "
        DELETE FROM posts
        WHERE createat < $1;
    ";
  trace!("Querying: {}", &query);
  trace!("Params: {:#?}", &millisecond_epoch);
  if dry_run {
    info!(
      "[DRY RUN] Would delete `posts` rows older than {}",
      millisecond_epoch
    );
    return Ok(());
  }
  let result = client
    .execute(query, &[&millisecond_epoch])
    .await
    .context("Failed to delete `posts` rows")?;
  info!("Removed {} post rows", result);
  Ok(())
}

fn validate(
  mattermost_data_directory: &str,
  database_name: &str,
  database_user: &str,
  database_host: &str,
  retention_days: i64,
  file_batch_size: usize,
) -> Result<()> {
  if mattermost_data_directory.is_empty()
    || database_name.is_empty()
    || database_user.is_empty()
    || database_host.is_empty()
    || retention_days <= 0
    || file_batch_size == 0
  {
    anyhow::bail!("Invalid input parameters");
  }
  Ok(())
}
