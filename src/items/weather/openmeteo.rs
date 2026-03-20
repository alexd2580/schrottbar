use std::time::Duration;

use log::warn;
use serde::Deserialize;

use super::WeatherData;
use crate::error::Error;

#[derive(Deserialize, Debug)]
struct Current {
    temperature_2m: f64,
    weather_code: u32,
}

#[derive(Deserialize, Debug)]
struct Daily {
    sunrise: Vec<String>,
    sunset: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct Response {
    current: Current,
    daily: Daily,
}

/// Parse an ISO8601 datetime like "2026-03-20T06:30" into duration since midnight.
fn iso_time_to_duration(iso: &str) -> chrono::Duration {
    let time_part = iso.split('T').nth(1).unwrap_or("12:00");
    let mut parts = time_part.split(':');
    let hours: i64 = parts.next().unwrap_or("12").parse().unwrap_or(12);
    let minutes: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    chrono::Duration::minutes(hours * 60 + minutes)
}

/// Map WMO weather code to a human-readable condition string.
fn wmo_condition(code: u32) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly Clear",
        2 => "Partly Cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing Drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing Rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow Grains",
        80..=82 => "Rain Showers",
        85 | 86 => "Snow Showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm w/ Hail",
        _ => "Unknown",
    }
}

use super::Location;

async fn run_request(loc: Location) -> Result<WeatherData, Error> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?\
         latitude={}&longitude={}\
         &current=temperature_2m,weather_code\
         &daily=sunrise,sunset\
         &timezone=auto\
         &forecast_days=1",
        loc.lat, loc.lon
    );
    let response = reqwest::Client::default()
        .get(&url)
        .timeout(Duration::new(5, 0))
        .send()
        .await?;
    let json = response.json::<Response>().await?;

    Ok(WeatherData {
        temp: json.current.temperature_2m.round() as i32,
        wmo_code: json.current.weather_code,
        condition: wmo_condition(json.current.weather_code).to_string(),
        midnight_to_sunrise: iso_time_to_duration(&json.daily.sunrise[0]),
        midnight_to_sunset: iso_time_to_duration(&json.daily.sunset[0]),
    })
}

pub async fn get_weather_data(location: Option<Location>) -> Option<WeatherData> {
    let loc = location?;
    match run_request(loc).await {
        Ok(result) => Some(result),
        Err(err) => {
            warn!("open-meteo: {err}");
            None
        }
    }
}
