use crate::memory::reminder::{ParsedReminder, ReminderType};
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike, Utc};
use regex::Regex;

pub struct ReminderParser;

impl ReminderParser {
    pub fn parse(text: &str, timezone: &str) -> Option<ParsedReminder> {
        // Parse recorrentes primeiro
        if let Some(reminder) = Self::parse_recurring(text, timezone) {
            return Some(reminder);
        }

        // Parse datas únicas
        if let Some(reminder) = Self::parse_single(text, timezone) {
            return Some(reminder);
        }

        None
    }

    fn parse_recurring(text: &str, timezone: &str) -> Option<ParsedReminder> {
        let patterns = [
            // Todo dia às X
            (
                Regex::new(r"(?i)(todo\s+dia|todos\s+os\s+dias|diariamente)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap(),
                "daily"
            ),
            // Toda segunda, terça, etc
            (
                Regex::new(r"(?i)(toda\s+)?(segunda|ter[cç]a|quarta|quinta|sexta|s[aá]bado|domingo)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap(),
                "weekly"
            ),
            // Toda semana
            (
                Regex::new(r"(?i)(toda\s+semana|semanalmente)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap(),
                "weekly_fixed"
            ),
        ];

        for (regex, pattern_type) in &patterns {
            if let Some(caps) = regex.captures(text) {
                let message = Self::extract_message(text, regex);

                let (cron, next_run) = match *pattern_type {
                    "daily" => {
                        let hour: u32 = caps.get(2)?.as_str().parse().ok()?;
                        let minute: u32 = caps
                            .get(3)
                            .map(|m| m.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(0);
                        let cron = format!("0 {} {} * * *", minute, hour);
                        let next = Self::calculate_next_daily(hour, minute, timezone)?;
                        (cron, next)
                    }
                    "weekly" => {
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
                        (cron, next)
                    }
                    _ => return None,
                };

                return Some(ParsedReminder {
                    message,
                    reminder_type: ReminderType::Recurring(cron),
                    datetime: Some(next_run),
                });
            }
        }

        None
    }

    fn parse_single(text: &str, timezone: &str) -> Option<ParsedReminder> {
        let patterns = [
            // Amanhã às X
            (
                Regex::new(r"(?i)(amanh[aã])\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap(),
                "tomorrow"
            ),
            // Hoje às X
            (
                Regex::new(r"(?i)(hoje)\s+[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?").unwrap(),
                "today"
            ),
            // Daqui X horas/minutos
            (
                Regex::new(r"(?i)(?:daqui\s+a)?\s*(\d+)\s*(hora|horas|min|minutos?)").unwrap(),
                "relative"
            ),
            // Data específica: DD/MM/YYYY ou DD/MM às X
            (
                Regex::new(r"(?i)(\d{1,2})[/-](\d{1,2})(?:[/-](\d{2,4}))?\s*(?:[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?)?").unwrap(),
                "absolute"
            ),
            // Em X dias
            (
                Regex::new(r"(?i)(?:em\s+)?(\d+)\s*dias?\s*(?:[aà]s\s+(\d{1,2})(?::(\d{2}))?(?:\s*h)?)?").unwrap(),
                "days"
            ),
        ];

        for (regex, pattern_type) in &patterns {
            if let Some(caps) = regex.captures(text) {
                let message = Self::extract_message(text, regex);

                let datetime = match *pattern_type {
                    "tomorrow" => {
                        let hour: u32 = caps.get(2)?.as_str().parse().ok()?;
                        let minute: u32 = caps
                            .get(3)
                            .map(|m| m.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(0);
                        Self::tomorrow_at(hour, minute, timezone)?
                    }
                    "today" => {
                        let hour: u32 = caps.get(2)?.as_str().parse().ok()?;
                        let minute: u32 = caps
                            .get(3)
                            .map(|m| m.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(0);
                        Self::today_at(hour, minute, timezone)?
                    }
                    "relative" => {
                        let amount: i64 = caps.get(1)?.as_str().parse().ok()?;
                        let unit = caps.get(2)?.as_str();
                        Self::relative_time(amount, unit, timezone)?
                    }
                    "absolute" => {
                        let day: u32 = caps.get(1)?.as_str().parse().ok()?;
                        let month: u32 = caps.get(2)?.as_str().parse().ok()?;
                        let year: Option<i32> = caps
                            .get(3)
                            .map(|y| {
                                let y: i32 = y.as_str().parse().ok()?;
                                if y < 100 {
                                    Some(2000 + y)
                                } else {
                                    Some(y)
                                }
                            })
                            .flatten();
                        let hour: u32 = caps
                            .get(4)
                            .map(|h| h.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(9);
                        let minute: u32 = caps
                            .get(5)
                            .map(|m| m.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(0);
                        Self::absolute_date(day, month, year, hour, minute, timezone)?
                    }
                    "days" => {
                        let days: i64 = caps.get(1)?.as_str().parse().ok()?;
                        let hour: u32 = caps
                            .get(2)
                            .map(|h| h.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(9);
                        let minute: u32 = caps
                            .get(3)
                            .map(|m| m.as_str().parse().ok())
                            .flatten()
                            .unwrap_or(0);
                        Self::days_from_now(days, hour, minute, timezone)?
                    }
                    _ => return None,
                };

                return Some(ParsedReminder {
                    message,
                    reminder_type: ReminderType::Single,
                    datetime: Some(datetime),
                });
            }
        }

        None
    }

    fn extract_message(text: &str, pattern: &Regex) -> String {
        // Remove a parte temporal do texto para extrair a mensagem
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

    // Helper functions for datetime calculations
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

        // Se já passou, agenda para amanhã
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

    fn absolute_date(
        day: u32,
        month: u32,
        year: Option<i32>,
        hour: u32,
        minute: u32,
        timezone: &str,
    ) -> Option<DateTime<Utc>> {
        let tz: chrono::FixedOffset = Self::parse_timezone(timezone)?;
        let current_year = Self::now_in_timezone(timezone)?.year();
        let y = year.unwrap_or(current_year);

        let naive = chrono::NaiveDate::from_ymd_opt(y, month, day)?.and_hms_opt(hour, minute, 0)?;

        Some(tz.from_local_datetime(&naive).single()?.with_timezone(&Utc))
    }

    fn days_from_now(days: i64, hour: u32, minute: u32, timezone: &str) -> Option<DateTime<Utc>> {
        let local = Self::now_in_timezone(timezone)?;
        let target_date = local + Duration::days(days);
        let target = target_date
            .with_hour(hour)?
            .with_minute(minute)?
            .with_second(0)?;
        Some(target.with_timezone(&Utc))
    }

    fn calculate_next_daily(hour: u32, minute: u32, timezone: &str) -> Option<DateTime<Utc>> {
        Self::today_at(hour, minute, timezone)
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
            // Mesmo dia, verifica se a hora já passou
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
        // Parse common timezone strings
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
                // Try to parse as offset like "-03:00" or "+05:30"
                if let Ok(offset) = Self::parse_offset(timezone) {
                    Some(offset)
                } else {
                    // Default to UTC
                    Some(chrono::FixedOffset::east_opt(0)?)
                }
            }
        }
    }

    fn parse_offset(offset: &str) -> Result<chrono::FixedOffset, ()> {
        // Parse strings like "-03:00", "+05:30", "-3", "+5:30"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tomorrow() {
        let result = ReminderParser::parse(
            "Me lembre de tomar remédio amanhã às 8h",
            "America/Sao_Paulo",
        );
        assert!(result.is_some());
        let reminder = result.unwrap();
        assert!(matches!(reminder.reminder_type, ReminderType::Single));
        assert!(reminder.datetime.is_some());
    }

    #[test]
    fn test_parse_daily() {
        let result = ReminderParser::parse(
            "Me lembre todo dia às 9h de tomar remédio",
            "America/Sao_Paulo",
        );
        assert!(result.is_some());
        let reminder = result.unwrap();
        assert!(matches!(reminder.reminder_type, ReminderType::Recurring(_)));
    }

    #[test]
    fn test_parse_relative_hours() {
        let result = ReminderParser::parse("Me lembre daqui 2 horas", "America/Sao_Paulo");
        assert!(result.is_some());
        let reminder = result.unwrap();
        assert!(matches!(reminder.reminder_type, ReminderType::Single));
    }
}
