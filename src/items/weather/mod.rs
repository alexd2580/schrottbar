use std::sync::Arc;

use crate::types::{PowerlineDirection, PowerlineStyle, RGBA};
use log::{debug, info};
use serde::Deserialize;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{BLUE, DARK_GREEN, RED, SectionWriter, THIN_SPACE, mix_colors_multi},
    state_item::{
        ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem, wait_seconds,
    },
    utils::time::{duration_since_midnight, split_duration},
};

mod openmeteo;
mod wttrin;

/// Shared weather data produced by any provider.
#[derive(Debug)]
pub struct WeatherData {
    pub temp: i32,
    /// WMO weather interpretation code (0–99).
    pub wmo_code: u32,
    pub condition: String,
    pub midnight_to_sunrise: chrono::Duration,
    pub midnight_to_sunset: chrono::Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}

/// Resolve location via IP geolocation (ip-api.com, no API key needed).
async fn geolocate() -> Option<Location> {
    #[derive(Deserialize)]
    struct GeoResponse {
        lat: f64,
        lon: f64,
    }

    let resp = reqwest::Client::default()
        .get("http://ip-api.com/json/?fields=lat,lon")
        .timeout(std::time::Duration::new(3, 0))
        .send()
        .await
        .ok()?;
    let geo: GeoResponse = resp.json().await.ok()?;
    info!("Geolocation: {:.2}, {:.2}", geo.lat, geo.lon);
    Some(Location {
        lat: geo.lat,
        lon: geo.lon,
    })
}

/// Fetch weather from all providers in parallel, return the first success.
async fn fetch_weather() -> Option<WeatherData> {
    let (a, b) = tokio::join!(wttrin::get_weather_data(), async {
        let location = geolocate().await;
        openmeteo::get_weather_data(location).await
    });
    // Prefer open-meteo (no IP-based geolocation quirks), fall back to wttr.in.
    if b.is_some() {
        info!("Weather from open-meteo");
        b
    } else if a.is_some() {
        info!("Weather from wttr.in");
        a
    } else {
        None
    }
}

type SharedData = Arc<Mutex<Option<WeatherData>>>;
pub struct Weather(SharedData);

impl Weather {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

fn weather_icon(data: &WeatherData, since_midnight: chrono::Duration) -> &'static str {
    match data.wmo_code {
        // Clear / sunny
        0 | 1 => {
            let is_day = since_midnight > data.midnight_to_sunrise
                && since_midnight < data.midnight_to_sunset;
            if is_day { "\u{f0599}" } else { "\u{f0594}" } // sunny / night
        }
        // Partly cloudy
        2 => "\u{f0595}",
        // Overcast
        3 => "\u{f0163}",
        // Fog
        45 | 48 => "\u{f0591}",
        // Drizzle
        51 | 53 | 55 | 56 | 57 => "\u{f0597}",
        // Rain (light/moderate)
        61 | 63 | 80 | 81 => "\u{f0597}",
        // Rain (heavy)
        65 | 82 => "\u{f0596}",
        // Freezing rain
        66 | 67 => "\u{f0592}",
        // Snow (light)
        71 | 77 | 85 => "\u{f0598}",
        // Snow (heavy)
        73 | 75 | 86 => "\u{f059a}",
        // Thunderstorm
        95 | 96 | 99 => "\u{f059e}",
        _ => "?",
    }
}

fn next_event(data: &WeatherData, since_midnight: chrono::Duration) -> (&str, chrono::Duration) {
    if since_midnight < data.midnight_to_sunrise {
        ("\u{f059c}", data.midnight_to_sunrise - since_midnight)
    } else if since_midnight < data.midnight_to_sunset {
        ("\u{f059b}", data.midnight_to_sunset - since_midnight)
    } else {
        let since_midnight = since_midnight - chrono::Duration::days(1);
        ("\u{f059c}", data.midnight_to_sunrise - since_midnight)
    }
}

pub const COLD: RGBA = BLUE;
pub const NORMAL: RGBA = DARK_GREEN;
pub const HOT: RGBA = RED;

const WEATHER_REFERENCE_POINTS: [(f32, RGBA); 4] =
    [(-5f32, COLD), (5f32, NORMAL), (20f32, NORMAL), (30f32, HOT)];

#[async_trait::async_trait]
impl StateItem for Weather {
    #[allow(clippy::cast_precision_loss)]
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Fade);
        writer.set_direction(PowerlineDirection::Left);

        let state = self.0.lock().await;
        if let Some(ref data) = *state {
            let temp_color = mix_colors_multi(data.temp as f32, &WEATHER_REFERENCE_POINTS);
            writer.with_bg(temp_color, &|writer| {
                let since_midnight = duration_since_midnight();
                let weather_icon = weather_icon(data, since_midnight);

                writer.write(format!(
                    "{weather_icon} {} {}°C{THIN_SPACE}",
                    data.condition, data.temp
                ));
                writer.split();

                let (icon, duration) = next_event(data, since_midnight);
                let (hours, minutes) = split_duration(duration);
                writer.write(format!("{icon} in {hours:0>2}:{minutes:0>2}{THIN_SPACE}"));
            });
        } else {
            writer.with_bg(RED, &|writer| {
                writer.write(format!("\u{f0164}{THIN_SPACE}"))
            });
        }
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(weather_coroutine(
            self.0.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn weather_coroutine(
    state: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    loop {
        {
            let new_state = fetch_weather().await;
            let mut state_lock = state.lock().await;
            *state_lock = new_state;
            if !main_action_sender.enqueue(MainAction::Redraw).await {
                break;
            }
        }

        tokio::select! {
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update)  => {},
                    Some(ItemAction::Terminate) => break,
                }
            }
            _ = wait_seconds(1800) => {}
        }
    }
    debug!("coroutine exiting");
}
