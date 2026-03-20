use chrono::FixedOffset;
use serde::Deserialize;

use crate::{error::Error, utils::config};

#[derive(Debug)]
pub struct PaymoData {
    pub api_key: Option<String>,
    pub user_id: Option<i64>,
    pub running_task: Option<(String, chrono::DateTime<FixedOffset>)>,
}

fn read_api_key() -> Result<String, Error> {
    let contents = config::read_config_file("paymo")?;
    Ok(contents.strip_suffix("\n").unwrap_or(&contents).to_string())
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct PaymoUser {
    id: i64,
    name: String,
    email: String,
    timezone: String,
}

/// Response to "/api/me".
#[derive(Deserialize, Debug)]
struct PaymoGetMe {
    users: Vec<PaymoUser>,
}

async fn request_user(api_key: &str) -> Result<PaymoUser, Error> {
    let url = format!("{PAYMO_BASE_URL}/api/me");
    let response = reqwest::Client::default()
        .get(url)
        .basic_auth(api_key, Some("SOME_RANDOM_TEXT"))
        .send()
        .await?;
    let json = response.json::<PaymoGetMe>().await?;
    Ok(json.users[0].clone())
}

const PAYMO_BASE_URL: &str = "https://app.paymoapp.com";

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct PaymoEntry {
    id: i64,
    task_id: i64,
    user_id: i64,
    start_time: String,
    end_time: Option<String>,
    description: Option<String>,
}

/// Response to "/api/entries".
#[derive(Deserialize, Debug)]
struct PaymoGetEntries {
    entries: Vec<PaymoEntry>,
}

async fn request_running_entry(api_key: &str, user_id: i64) -> Result<Option<PaymoEntry>, Error> {
    let url = format!("{PAYMO_BASE_URL}/api/entries");
    let response = reqwest::Client::default()
        .get(url)
        .query(&[("where", format!("user_id={user_id} and end_time=null"))])
        .basic_auth(api_key, Some("SOME_RANDOM_TEXT"))
        .send()
        .await?;
    let json = response.json::<PaymoGetEntries>().await?;
    Ok(json.entries.first().cloned())
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct PaymoTask {
    id: i64,
    name: String,
}

/// Response to "/api/tasks/[id]"
#[derive(Deserialize, Debug)]
struct PaymoGetTasks {
    tasks: Vec<PaymoTask>,
}

async fn request_task(api_key: &str, task_id: i64) -> Result<PaymoTask, Error> {
    let url = format!("{PAYMO_BASE_URL}/api/tasks/{task_id}");
    let response = reqwest::Client::default()
        .get(url)
        .basic_auth(api_key, Some("SOME_RANDOM_TEXT"))
        .send()
        .await?;
    let json = response.json::<PaymoGetTasks>().await?;
    Ok(json.tasks[0].clone())
}

async fn run_requests(old_data: &Option<PaymoData>) -> Result<PaymoData, Error> {
    let api_key = match old_data {
        Some(PaymoData {
            api_key: Some(api_key),
            ..
        }) => Ok(api_key.to_string()),
        _ => read_api_key(),
    }?;
    let user_id = match old_data {
        Some(PaymoData {
            user_id: Some(user_id),
            ..
        }) => Ok::<_, Error>(*user_id),
        _ => {
            let user = request_user(&api_key).await?;
            Ok(user.id)
        }
    }?;
    let with_no_timer = PaymoData {
        api_key: Some(api_key.clone()),
        user_id: Some(user_id),
        running_task: None,
    };
    Ok(match request_running_entry(&api_key, user_id).await? {
        None => with_no_timer,
        Some(current_entry) => {
            let current_task = request_task(&api_key, current_entry.task_id).await?;
            let as_datetime = chrono::DateTime::parse_from_rfc3339(&current_entry.start_time)?;
            PaymoData {
                running_task: Some((current_task.name, as_datetime)),
                ..with_no_timer
            }
        }
    })
}

pub async fn query_running_paymo_task(old_data: &Option<PaymoData>) -> Option<PaymoData> {
    // Config missing or API error — expected if paymo isn't set up.
    (run_requests(old_data).await).ok()
}
