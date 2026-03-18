use std::time::Duration;

use log::warn;
use serde::Deserialize;

use crate::error::Error;

#[derive(Debug)]
pub struct WeatherData {
    pub temp: i32,
    pub condition_code: u32,
    pub condition: String,

    pub midnight_to_sunrise: chrono::Duration,
    pub midnight_to_sunset: chrono::Duration,
}

#[derive(Deserialize, Debug)]
struct WttrInFormatJ1CurrentConditionWeatherDesc {
    value: String,
}

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
struct WttrInFormatJ1CurrentCondition {
    temp_C: String,
    weatherCode: String,
    weatherDesc: Vec<WttrInFormatJ1CurrentConditionWeatherDesc>,
}

#[derive(Deserialize, Debug)]
struct WttrInFormatJ1WeatherAstronomy {
    sunrise: String,
    sunset: String,
}

#[derive(Deserialize, Debug)]
struct WttrInFormatJ1Weather {
    astronomy: Vec<WttrInFormatJ1WeatherAstronomy>,
}

#[derive(Deserialize, Debug)]
struct WttrInFormatJ1Data {
    current_condition: Vec<WttrInFormatJ1CurrentCondition>,
    weather: Vec<WttrInFormatJ1Weather>,
}

#[derive(Deserialize, Debug)]
struct WttrInFormatJ1 {
    data: WttrInFormatJ1Data,
}

fn am_pm_to_duration_since_midnight(text: &str) -> chrono::Duration {
    let bytes = text.as_bytes();
    // The ascii byte value of '0' is 48.
    let mut hours = 10 * (i64::from(bytes[0]) - 48) + (i64::from(bytes[1]) - 48);
    let minutes = 10 * (i64::from(bytes[3]) - 48) + (i64::from(bytes[4]) - 48);

    // The ascii byte value of 'P' is 80.
    if bytes[6] == 80 {
        hours += 12;
    }

    chrono::Duration::minutes(hours * 60 + minutes)
}

fn handle_response(response: &WttrInFormatJ1) -> WeatherData {
    let current_condition = &response.data.current_condition[0];
    let astronomy = &response.data.weather[0].astronomy[0];
    WeatherData {
        temp: current_condition.temp_C.parse().unwrap(),
        condition_code: current_condition.weatherCode.parse().unwrap(),
        condition: current_condition.weatherDesc[0].value.clone(),
        midnight_to_sunrise: am_pm_to_duration_since_midnight(&astronomy.sunrise),
        midnight_to_sunset: am_pm_to_duration_since_midnight(&astronomy.sunset),
    }
}

async fn run_request() -> Result<WeatherData, Error> {
    let response = reqwest::Client::default()
        .get("https://wttr.in?format=j1")
        .timeout(Duration::new(3, 0))
        .send()
        .await?;
    let json = response.json::<WttrInFormatJ1>().await?;
    Ok(handle_response(&json))
}

pub async fn get_weather_data() -> Option<WeatherData> {
    match run_request().await {
        Ok(result) => Some(result),
        Err(err) => {
            warn!("{err}");
            None
        }
    }
}
