use chrono::Timelike;

pub fn duration_since_midnight() -> chrono::TimeDelta {
    let secs_from_midnight = chrono::Local::now().num_seconds_from_midnight();
    chrono::TimeDelta::seconds(i64::from(secs_from_midnight))
}

pub fn split_duration(duration: chrono::TimeDelta) -> (i64, i64) {
    let hours = duration.num_hours();
    let minutes = (duration - chrono::TimeDelta::hours(hours)).num_minutes();
    (hours, minutes)
}
