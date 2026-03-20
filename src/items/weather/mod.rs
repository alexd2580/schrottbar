use std::sync::Arc;

use crate::types::{PowerlineDirection, PowerlineStyle, RGBA};
use log::debug;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{BLUE, DARK_GREEN, RED, SectionWriter, THIN_SPACE, mix_colors_multi},
    state_item::{
        ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem, wait_seconds,
    },
    utils::time::{duration_since_midnight, split_duration},
};

use self::wttrin::{WeatherData, get_weather_data};

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
        // nf-md-weather_lightning (thunder)
        389 | 386 | 200 => "\u{f059e}",

        // nf-md-weather_rainy (light rain/drizzle)
        266 | 263 | 293 | 176 | 296 | 353 => "\u{f0597}",

        // nf-md-weather_pouring (heavy rain)
        302 | 299 | 356 | 308 | 305 | 359 => "\u{f0596}",

        // nf-md-weather_snowy (light snow)
        179 | 323 | 326 | 368 => "\u{f0598}",

        // nf-md-weather_snowy_heavy (heavy snow/blizzard)
        395 | 392 | 329 | 332 | 338 | 371 | 335 | 227 | 230 => "\u{f059a}",

        // nf-md-weather_hail (sleet/ice)
        365 | 362 | 350 | 320 | 317 | 185 | 182 | 377 | 311 | 374 | 284 | 281 | 314 => "\u{f0592}",

        // nf-md-weather_fog
        260 | 248 | 143 => "\u{f0591}",

        // nf-md-weather_cloudy (overcast/cloudy)
        122 | 119 => "\u{f0163}",

        // nf-md-weather_partly_cloudy (partly cloudy)
        116 => "\u{f0595}",

        // nf-md-weather_sunny / nf-md-weather_night
        113 => {
            let is_day = since_midnight > data.midnight_to_sunrise
                && since_midnight < data.midnight_to_sunset;

            if is_day { "\u{f0599}" } else { "\u{f0594}" }
        }
        _ => "?",
    }
}

fn next_event(data: &WeatherData, since_midnight: chrono::Duration) -> (&str, chrono::Duration) {
    if since_midnight < data.midnight_to_sunrise {
        ("\u{f059c}", data.midnight_to_sunrise - since_midnight) // nf-md-weather_sunset_up
    } else if since_midnight < data.midnight_to_sunset {
        ("\u{f059b}", data.midnight_to_sunset - since_midnight) // nf-md-weather_sunset_down
    } else {
        let since_midnight = since_midnight - chrono::Duration::days(1);
        ("\u{f059c}", data.midnight_to_sunrise - since_midnight) // nf-md-weather_sunset_up
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
