use crate::{error::Error, utils::config};

#[derive(Debug)]
pub struct Shortener {
    browsers: Vec<(String, String)>,
    websites: Vec<(String, String)>,
    editors: Vec<(String, String)>,
    languages: Vec<(String, String)>,
    programs: Vec<(String, String)>,
}

impl Shortener {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            browsers: Self::load_list("browsers")?,
            websites: Self::load_list("websites")?,
            editors: Self::load_list("editors")?,
            languages: Self::load_list("languages")?,
            programs: Self::load_list("programs")?,
        })
    }

    fn load_list(list: &str) -> Result<Vec<(String, String)>, Error> {
        let content = config::read_config_file(list)?;
        let re = regex::Regex::new(r"^(?<match>.*?)\s*=\s*(?<icon>.*?)$").unwrap();
        content
            .lines()
            .map(|line| {
                re.captures(line)
                    .ok_or_else(|| {
                        Error::Local(format!("Line \"{line}\" does not match expected format"))
                    })
                    .map(|captures| {
                        (
                            captures.name("match").unwrap().as_str().to_owned(),
                            captures.name("icon").unwrap().as_str().to_owned(),
                        )
                    })
            })
            .collect::<Result<_, _>>()
    }

    pub fn shorten(&self, title: &str) -> String {
        let title = title.to_lowercase();
        for (browser, browser_icon) in &self.browsers {
            if title.contains(browser) {
                for (site, site_icon) in &self.websites {
                    if title.contains(site) {
                        return format!("{browser_icon}{site_icon}");
                    }
                }
                return browser_icon.to_string();
            }
        }

        for (editor, editor_icon) in &self.editors {
            if title.contains(editor) {
                for (lang, lang_icon) in &self.languages {
                    let re = regex::Regex::new(&format!(r"{lang}[\s$\W]")).unwrap();
                    if re.is_match(&title) {
                        return format!("{editor_icon}{lang_icon}");
                    }
                }
                return editor_icon.to_string();
            }
        }

        for (prgm, prgm_icon) in &self.programs {
            if title.contains(prgm) {
                return prgm_icon.to_string();
            }
        }

        if title.len() > 20 {
            title.chars().take(18).collect()
        } else {
            title.to_string()
        }
    }
}
