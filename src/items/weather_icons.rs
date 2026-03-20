use tokio::task::JoinHandle;

use crate::{
    error::Error,
    section_writer::{DARK_GRAY, SectionWriter},
    state_item::{ItemAction, ItemActionReceiver, MainActionSender, StateItem},
    types::{PowerlineDirection, PowerlineStyle},
};

#[allow(dead_code)]
pub struct WeatherIcons;

impl WeatherIcons {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }
}

#[allow(dead_code)]
const NF_WEATHER: &[(&str, &str)] = &[
    ("sunny", "\u{e30d}"),
    ("night", "\u{e32b}"),
    ("cloud", "\u{e312}"),
    ("fog", "\u{e313}"),
    ("shower", "\u{e319}"),
    ("rain", "\u{e318}"),
    ("thunder", "\u{e31d}"),
    ("snow", "\u{e31a}"),
    ("snow+w", "\u{e35e}"),
    ("sleet", "\u{e3ad}"),
    // Extra nf-weather icons worth considering
    ("day-cld", "\u{e302}"),
    ("nit-cld", "\u{e37e}"),
    ("day-fog", "\u{e303}"),
    ("day-haz", "\u{e304}"),
    ("day-rain", "\u{e309}"),
    ("nit-rain", "\u{e325}"),
    ("day-snow", "\u{e30a}"),
    ("day-thun", "\u{e30e}"),
    ("nit-thun", "\u{e32a}"),
    ("windy", "\u{e34b}"),
    ("hail", "\u{e314}"),
    ("sunrise", "\u{e34c}"),
    ("sunset", "\u{e34d}"),
    ("thermo", "\u{e350}"),
];

#[allow(dead_code)]
const NF_MD: &[(&str, &str)] = &[
    ("sunny", "\u{f0599}"),
    ("night", "\u{f0594}"),
    ("cloud", "\u{f0163}"),
    ("fog", "\u{f0591}"),
    ("rain", "\u{f0597}"),
    ("pour", "\u{f0596}"),
    ("thunder", "\u{f059e}"),
    ("snow", "\u{f0598}"),
    ("snowy", "\u{f059a}"),
    ("hail", "\u{f0592}"),
    ("windy", "\u{f059d}"),
    ("part-cld", "\u{f0595}"),
    ("nit-pcld", "\u{f0F31}"),
    ("sunset-u", "\u{f059c}"),
    ("sunset-d", "\u{f059b}"),
    ("thermo", "\u{f050f}"),
    ("alert", "\u{f0f2f}"),
    ("no-wthr", "\u{f0164}"),
];

#[allow(dead_code)]
const EMOJI: &[(&str, &str)] = &[
    ("sun", "☀"),
    ("cloud", "☁"),
    ("umbrella", "☂"),
    ("snowman", "☃"),
    ("comet", "☄"),
    ("thunder", "⛈"),
    ("sun-cld", "⛅"),
    ("fog", "🌫"),
    ("rainbow", "🌈"),
    ("tornado", "🌪"),
];

#[async_trait::async_trait]
impl StateItem for WeatherIcons {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Left);

        writer.with_bg(DARK_GRAY, &|writer| {
            for (_label, icon) in NF_MD {
                writer.write(format!(" {icon}"));
            }
            writer.write(" ".to_string());
        });

        Ok(())
    }

    fn start_coroutine(
        &self,
        _main_action_sender: MainActionSender,
        mut item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = item_action_receiver.next() => {
                        match message {
                            None | Some(ItemAction::Update) => {}
                            Some(ItemAction::Terminate) => break,
                        }
                    }
                }
            }
        })
    }
}
