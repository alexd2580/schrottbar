use std::time::Duration;

use log::warn;
use serde::Deserialize;

use super::WeatherData;
use crate::error::Error;

#[derive(Deserialize, Debug)]
struct WeatherDesc {
    value: String,
}

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
struct CurrentCondition {
    temp_C: String,
    weatherCode: String,
    weatherDesc: Vec<WeatherDesc>,
}

#[derive(Deserialize, Debug)]
struct Astronomy {
    sunrise: String,
    sunset: String,
}

#[derive(Deserialize, Debug)]
struct Weather {
    astronomy: Vec<Astronomy>,
}

#[derive(Deserialize, Debug)]
struct Response {
    current_condition: Vec<CurrentCondition>,
    weather: Vec<Weather>,
}

fn am_pm_to_duration_since_midnight(text: &str) -> chrono::Duration {
    let bytes = text.as_bytes();
    let mut hours = 10 * (i64::from(bytes[0]) - 48) + (i64::from(bytes[1]) - 48);
    let minutes = 10 * (i64::from(bytes[3]) - 48) + (i64::from(bytes[4]) - 48);
    if bytes[6] == b'P' {
        hours += 12;
    }
    chrono::Duration::minutes(hours * 60 + minutes)
}

/// Convert WWO (World Weather Online) condition codes to WMO codes.
fn wwo_to_wmo(wwo: u32) -> u32 {
    match wwo {
        113 => 0,              // Clear
        116 => 2,              // Partly cloudy
        119 => 3,              // Cloudy / Overcast
        122 => 3,              // Overcast
        143 => 45,             // Mist / Fog
        176 | 263 | 266 => 51, // Light drizzle
        179 | 323 | 326 => 71, // Light snow
        182 | 185 => 66,       // Freezing drizzle/sleet
        200 | 386 | 389 => 95, // Thunderstorm
        248 | 260 => 45,       // Fog
        293 | 296 | 353 => 61, // Light rain
        299 | 302 | 356 => 63, // Moderate rain
        305 | 308 | 359 => 65, // Heavy rain
        311 | 314 => 67,       // Freezing rain
        317 | 320 => 66,       // Sleet
        329 | 332 => 73,       // Moderate snow
        335 | 338 => 75,       // Heavy snow
        227 | 230 => 75,       // Blizzard
        350 | 362 | 365 | 374 | 377 => 77, // Ice / hail
        368 | 371 => 85,       // Snow showers
        392 | 395 => 96,       // Thunderstorm with hail/snow
        281 | 284 => 67,       // Freezing rain
        _ => 3,                // Default to overcast
    }
}

async fn run_request() -> Result<WeatherData, Error> {
    let response = reqwest::Client::default()
        .get("https://wttr.in?format=j1")
        .timeout(Duration::new(3, 0))
        .send()
        .await?;
    let json = response.json::<Response>().await?;

    let cc = &json.current_condition[0];
    let astro = &json.weather[0].astronomy[0];
    let wwo_code: u32 = cc.weatherCode.parse().unwrap_or(0);

    Ok(WeatherData {
        temp: cc.temp_C.parse().unwrap_or(0),
        wmo_code: wwo_to_wmo(wwo_code),
        condition: cc.weatherDesc[0].value.clone(),
        midnight_to_sunrise: am_pm_to_duration_since_midnight(&astro.sunrise),
        midnight_to_sunset: am_pm_to_duration_since_midnight(&astro.sunset),
    })
}

pub async fn get_weather_data() -> Option<WeatherData> {
    match run_request().await {
        Ok(result) => Some(result),
        Err(err) => {
            warn!("wttr.in: {err}");
            None
        }
    }
}
