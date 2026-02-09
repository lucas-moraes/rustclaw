use crate::memory::reminder::{ParsedReminder, ReminderType};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use regex::Regex;

pub struct ReminderParser;

impl ReminderParser {
    pub fn parse(text: &str, timezone: &str) -> Option<ParsedReminder> {
        if let Some(reminder) = Self::parse_recurring(text, timezone) {
            return Some(reminder);
        }

        if let Some(reminder) = Self::parse_single(text, timezone) {
            return Some(reminder);
        }

        None
    }

    fn parse_recurring(text: &str, timezone: &str) -> Option<ParsedReminder> {
        let daily_pattern = Regex::new(r"(?i)(todo\s+dia|todos\s+os\s+dias|diariamente)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap();

        if let Some(caps) = daily_pattern.captures(text) {
            let message = Self::extract_message(text, &daily_pattern);
            let hour: u32 = caps.get(2)?.as_str().parse().ok()?;
            let minute: u32 = caps
                .get(3)
                .map(|m| m.as_str().parse().ok())
                .flatten()
                .unwrap_or(0);
            let cron = format!("0 {} {} * * *", minute, hour);
            let next = Self::today_at(hour, minute, timezone)?;

            return Some(ParsedReminder {
                message,
                reminder_type: ReminderType::Recurring(cron),
                datetime: Some(next),
            });
        }

        let weekly_pattern = Regex::new(r"(?i)(toda\s+)?(segunda|ter[cç]a|quarta|quinta|sexta|s[aá]bado|domingo)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap();

        if let Some(caps) = weekly_pattern.captures(text) {
            let message = Self::extract_message(text, &weekly_pattern);
            let day_str = caps.get(2)?.as_str();
            let hour: u32 = caps.get(3)?.as_str().parse().ok()?;
            let minute: u32 = caps
                .get(4)
                .map(|m| m.as_str().parse().ok())
                .flatten()
                .unwrap_or(0);
            let cron_day = Self::day_to_cron(day_str)?;
            let cron = format!("0 {} {} * * {}", minute, hour, cron_day);
            let next = Self::calculate_next_weekly(&cron_day, hour, minute, timezone)?;

            return Some(ParsedReminder {
                message,
                reminder_type: ReminderType::Recurring(cron),
                datetime: Some(next),
            });
        }

        None
    }

    fn parse_single(text: &str, timezone: &str) -> Option<ParsedReminder> {
        let tomorrow_pattern =
            Regex::new(r"(?i)(amanh[aã])\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap();

        if let Some(caps) = tomorrow_pattern.captures(text) {
            let message = Self::extract_message(text, &tomorrow_pattern);
            let hour: u32 = caps.get(2)?.as_str().parse().ok()?;
            let minute: u32 = caps
                .get(3)
                .map(|m| m.as_str().parse().ok())
                .flatten()
                .unwrap_or(0);
            let datetime = Self::tomorrow_at(hour, minute, timezone)?;

            return Some(ParsedReminder {
                message,
                reminder_type: ReminderType::Single,
                datetime: Some(datetime),
            });
        }

        let today_pattern =
            Regex::new(r"(?i)(hoje)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap();

        if let Some(caps) = today_pattern.captures(text) {
            let message = Self::extract_message(text, &today_pattern);
            let hour: u32 = caps.get(2)?.as_str().parse().ok()?;
            let minute: u32 = caps
                .get(3)
                .map(|m| m.as_str().parse().ok())
                .flatten()
                .unwrap_or(0);
            let datetime = Self::today_at(hour, minute, timezone)?;

            return Some(ParsedReminder {
                message,
                reminder_type: ReminderType::Single,
                datetime: Some(datetime),
            });
        }

        let relative_pattern =
            Regex::new(r"(?i)(?:daqui\s+a)?\s*(\d+)\s*(hora|horas|min|minutos?)").unwrap();

        if let Some(caps) = relative_pattern.captures(text) {
            let message = Self::extract_message(text, &relative_pattern);
            let amount: i64 = caps.get(1)?.as_str().parse().ok()?;
            let unit = caps.get(2)?.as_str();
            let datetime = Self::relative_time(amount, unit, timezone)?;

            return Some(ParsedReminder {
                message,
                reminder_type: ReminderType::Single,
                datetime: Some(datetime),
            });
        }

        None
    }

    fn extract_message(text: &str, pattern: &Regex) -> String {
        let cleaned = pattern.replace(text, "");
        let cleaned = cleaned.replace("me lembre", "");
        let cleaned = cleaned.replace("Me lembre", "");
        let cleaned = cleaned.replace("de ", "");
        let cleaned = cleaned.trim();

        if cleaned.is_empty() {
            "Lembrete".to_string()
        } else {
            cleaned.to_string()
        }
    }

    fn day_to_cron(day: &str) -> Option<String> {
        match day.to_lowercase().as_str() {
            "domingo" => Some("0".to_string()),
            "segunda" | "segunda-feira" => Some("1".to_string()),
            "terça" | "terca" | "terça-feira" | "terca-feira" => Some("2".to_string()),
            "quarta" | "quarta-feira" => Some("3".to_string()),
            "quinta" | "quinta-feira" => Some("4".to_string()),
            "sexta" | "sexta-feira" => Some("5".to_string()),
            "sábado" | "sabado" => Some("6".to_string()),
            _ => None,
        }
    }

    fn tomorrow_at(hour: u32, minute: u32, timezone: &str) -> Option<DateTime<Utc>> {
        let local = Self::now_in_timezone(timezone)?;
        let tomorrow = local + Duration::days(1);
        let target = tomorrow
            .with_hour(hour)?
            .with_minute(minute)?
            .with_second(0)?;
        Some(target.with_timezone(&Utc))
    }

    fn today_at(hour: u32, minute: u32, timezone: &str) -> Option<DateTime<Utc>> {
        let local = Self::now_in_timezone(timezone)?;
        let target = local.with_hour(hour)?.with_minute(minute)?.with_second(0)?;

        if target < local {
            return Self::tomorrow_at(hour, minute, timezone);
        }

        Some(target.with_timezone(&Utc))
    }

    fn relative_time(amount: i64, unit: &str, timezone: &str) -> Option<DateTime<Utc>> {
        let local = Self::now_in_timezone(timezone)?;
        let duration = if unit.starts_with("hora") {
            Duration::hours(amount)
        } else {
            Duration::minutes(amount)
        };
        Some((local + duration).with_timezone(&Utc))
    }

    fn calculate_next_weekly(
        day: &str,
        hour: u32,
        minute: u32,
        timezone: &str,
    ) -> Option<DateTime<Utc>> {
        let local = Self::now_in_timezone(timezone)?;
        let target_day: u32 = day.parse().ok()?;
        let current_day = local.weekday().num_days_from_sunday();

        let days_diff = if target_day > current_day {
            (target_day - current_day) as i64
        } else if target_day < current_day {
            (7 - current_day + target_day) as i64
        } else {
            let target = local.with_hour(hour)?.with_minute(minute)?;
            if target > local {
                0
            } else {
                7
            }
        };

        let target_date = local + Duration::days(days_diff);
        let target = target_date
            .with_hour(hour)?
            .with_minute(minute)?
            .with_second(0)?;
        Some(target.with_timezone(&Utc))
    }

    fn now_in_timezone(timezone: &str) -> Option<DateTime<chrono::FixedOffset>> {
        let tz = Self::parse_timezone(timezone)?;
        Some(Utc::now().with_timezone(&tz))
    }

    fn parse_timezone(timezone: &str) -> Option<chrono::FixedOffset> {
        match timezone {
            "America/Sao_Paulo" | "America/Buenos_Aires" => {
                Some(chrono::FixedOffset::west_opt(3 * 3600)?)
            }
            "America/New_York" => Some(chrono::FixedOffset::west_opt(5 * 3600)?),
            "America/Chicago" => Some(chrono::FixedOffset::west_opt(6 * 3600)?),
            "America/Denver" => Some(chrono::FixedOffset::west_opt(7 * 3600)?),
            "America/Los_Angeles" => Some(chrono::FixedOffset::west_opt(8 * 3600)?),
            "Europe/London" => Some(chrono::FixedOffset::west_opt(0)?),
            "Europe/Paris" | "Europe/Berlin" => Some(chrono::FixedOffset::east_opt(1 * 3600)?),
            "Europe/Moscow" => Some(chrono::FixedOffset::east_opt(3 * 3600)?),
            "Asia/Tokyo" => Some(chrono::FixedOffset::east_opt(9 * 3600)?),
            "Asia/Shanghai" => Some(chrono::FixedOffset::east_opt(8 * 3600)?),
            "Australia/Sydney" => Some(chrono::FixedOffset::east_opt(10 * 3600)?),
            "UTC" => Some(chrono::FixedOffset::east_opt(0)?),
            _ => {
                if let Ok(offset) = Self::parse_offset(timezone) {
                    Some(offset)
                } else {
                    Some(chrono::FixedOffset::east_opt(0)?)
                }
            }
        }
    }

    fn parse_offset(offset: &str) -> Result<chrono::FixedOffset, ()> {
        let re = Regex::new(r"^([+-])(\d{1,2}):?(\d{2})?$").map_err(|_| ())?;
        if let Some(caps) = re.captures(offset) {
            let sign = caps.get(1).ok_or(())?.as_str();
            let hours: i32 = caps.get(2).ok_or(())?.as_str().parse().map_err(|_| ())?;
            let minutes: i32 = caps
                .get(3)
                .map(|m| m.as_str().parse().unwrap_or(0))
                .unwrap_or(0);

            let total_seconds = hours * 3600 + minutes * 60;

            if sign == "+" {
                chrono::FixedOffset::east_opt(total_seconds).ok_or(())
            } else {
                chrono::FixedOffset::west_opt(total_seconds).ok_or(())
            }
        } else {
            Err(())
        }
    }
}
