use std::sync::Arc;

use log::debug;
use crate::types::{
    PowerlineDirection, PowerlineStyle, RGBA,
};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    section_writer::{mix_colors_multi, SectionWriter, BLUE, DARK_GREEN, RED, THIN_SPACE},
    error::Error,
    state_item::{
        wait_seconds, ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem,
    },
    utils::time::{duration_since_midnight, split_duration},
};

use self::wttrin::{get_weather_data, WeatherData};

mod wttrin;

type SharedData = Arc<Mutex<Option<WeatherData>>>;
pub struct Weather(SharedData);

impl Weather {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

fn weather_icon(data: &WeatherData, since_midnight: chrono::Duration) -> &'static str {
    match data.condition_code {
        // nf-weather-thunderstorm
        // 389 Moderate or heavy rain in area with thunder
        // 386 Patchy light rain in area with thunder
        // 200 Thundery outbreaks in nearby
        389 | 386 | 200 => "\u{e31d}",

        // nf-weather-showers
        // 266 Light drizzle
        // 263 Patchy light drizzle
        // 293 Patchy light rain
        // 176 Patchy rain nearby
        // 296 Light rain
        // 353 Light rain shower
        266 | 263 | 293 | 176 | 296 | 353 => "\u{e319}",

        // nf-weather-rain
        // 302 Moderate rain
        // 299 Moderate rain at times
        // 356 Moderate or heavy rain shower
        // 308 Heavy rain
        // 305 Heavy rain at times
        // 359 Torrential rain shower
        302 | 299 | 356 | 308 | 305 | 359 => "\u{e318}",

        // nf-weather-snow
        // 179 Patchy snow nearby
        // 323 Patchy light snow
        // 326 Light snow
        // 368 Light snow showers
        179 | 323 | 326 | 368 => "\u{e31a}",

        // nf-weather-snow_wind
        // 395 Moderate or heavy snow in area with thunder
        // 392 Patchy light snow in area with thunder
        // 329 Patchy moderate snow
        // 332 Moderate snow
        // 338 Heavy snow
        // 371 Moderate or heavy snow showers
        // 335 Patchy heavy snow
        // 227 Blowing snow
        // 230 Blizzard
        395 | 392 | 329 | 332 | 338 | 371 | 335 | 227 | 230 => "\u{e35e}",

        // nf-weather-sleet
        // 365 Moderate or heavy sleet showers
        // 362 Light sleet showers
        // 350 Ice pellets
        // 320 Moderate or heavy sleet
        // 317 Light sleet
        // 185 Patchy freezing drizzle nearby
        // 182 Patchy sleet nearby
        // 377 Moderate or heavy showers of ice pellets
        // 311 Light freezing rain
        // 374 Light showers of ice pellets
        // 284 Heavy freezing drizzle  w
        // 281 Freezing drizzle
        // 314 Moderate or Heavy freezing rain
        365 | 362 | 350 | 320 | 317 | 185 | 182 | 377 | 311 | 374 | 284 | 281 | 314 => "\u{e3ad}",

        // nf-weather-fog
        // 260 Freezing fog
        // 248 Fog
        // 143 Mist
        260 | 248 | 143 => "\u{e313}",

        // nf-weather-cloud
        // 122 Overcast
        // 119 Cloudy
        // 116 Partly Cloudy
        122 | 119 | 116 => "\u{e312}",
        // nf-weather-night_clear
        // nf-weather-day_sunny
        // 113 Clear/Sunny
        113 => {
            let is_day = since_midnight > data.midnight_to_sunrise
                && since_midnight < data.midnight_to_sunset;

            if is_day {
                "\u{e30d}"
            } else {
                "\u{e32b}"
            }
        }
        _ => "unknown",
    }
}

fn next_event(data: &WeatherData, since_midnight: chrono::Duration) -> (&str, chrono::Duration) {
    if since_midnight < data.midnight_to_sunrise {
        ("\u{e34c}", data.midnight_to_sunrise - since_midnight)
    } else if since_midnight < data.midnight_to_sunset {
        ("\u{e34d}", data.midnight_to_sunset - since_midnight)
    } else {
        let since_midnight = since_midnight - chrono::Duration::days(1);
        ("\u{e34c}", data.midnight_to_sunrise - since_midnight)
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
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Left);

        let state = self.0.lock().await;
        if let Some(ref data) = *state {
            let temp_color = mix_colors_multi(data.temp as f32, &WEATHER_REFERENCE_POINTS);
            writer.with_bg(temp_color, &|writer| {
                let since_midnight = duration_since_midnight();
                let weather_icon = weather_icon(data, since_midnight);

                writer.write(format!(
                    "{weather_icon} {} {}{THIN_SPACE}",
                    data.condition, data.temp
                ));
                writer.split();

                let (icon, duration) = next_event(data, since_midnight);
                let (hours, minutes) = split_duration(duration);
                writer.write(format!("{icon} in {hours:0>2}:{minutes:0>2}{THIN_SPACE}"));
            });
        } else {
            writer.with_bg(RED, &|writer| writer.write(format!("\u{f0164}{THIN_SPACE}")));
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
            let new_state = get_weather_data().await;
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
