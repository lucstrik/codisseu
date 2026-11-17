//! Calendar periods use local dates; instants are exact, exclusive upper bounds.
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Month,
    Week,
    Day,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Month => "Mês",
            Self::Week => "Semana",
            Self::Day => "Dia",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Period {
    pub mode: Mode,
    pub anchor: String,
}
impl Default for Period {
    fn default() -> Self {
        Self {
            mode: Mode::Month,
            anchor: today(),
        }
    }
}
pub fn today() -> String {
    Local::now().date_naive().to_string()
}
pub fn parse_date(s: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| "Informe uma data válida")?;
    if !(1971..=9998).contains(&date.year()) {
        return Err("Data fora do intervalo1971–9998".into());
    }
    Ok(date)
}
pub fn typed_date(s: &str) -> Result<String, String> {
    for format in ["%d/%m/%Y", "%d-%m-%Y", "%Y-%m-%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s.trim(), format) {
            parse_date(&d.to_string())?;
            return Ok(d.to_string());
        }
    }
    Err("Use uma data válida no formato dd/mm/aaaa".into())
}
pub fn month_bounds(month: &str) -> Result<(String, String), String> {
    if month.len() != 7 {
        return Err("Use mês no formato aaaa-mm".into());
    }
    let d = parse_date(&format!("{month}-01"))?;
    let next = if d.month() == 12 {
        NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
    }
    .ok_or("Mês inválido")?;
    Ok((d.to_string(), next.to_string()))
}
impl Period {
    pub fn validate(&self) -> Result<(), String> {
        parse_date(&self.anchor)?;
        Ok(())
    }
    pub fn dates(&self) -> Result<(NaiveDate, NaiveDate), String> {
        let d = parse_date(&self.anchor)?;
        match self.mode {
            Mode::Month => {
                let (a, b) = month_bounds(&d.format("%Y-%m").to_string())?;
                Ok((parse_date(&a)?, parse_date(&b)?))
            }
            Mode::Week => {
                let start = d - Duration::days(i64::from(d.weekday().num_days_from_monday()));
                Ok((start, start + Duration::days(7)))
            }
            Mode::Day => Ok((d, d + Duration::days(1))),
        }
    }
    pub fn instants(&self) -> Result<(i64, i64), String> {
        let (a, b) = self.dates()?;
        Ok((midnight(a)?, midnight(b)?))
    }
    pub fn label(&self) -> String {
        let Ok((a, b)) = self.dates() else {
            return "Período inválido".into();
        };
        match self.mode {
            Mode::Month => format!("{} de {}", month_name(a.month()), a.year()),
            Mode::Week => format!(
                "{} – {}",
                a.format("%d/%m"),
                (b - Duration::days(1)).format("%d/%m/%Y")
            ),
            Mode::Day => format!(
                "{} de {} de {}",
                a.day(),
                month_name(a.month()).to_lowercase(),
                a.year()
            ),
        }
    }
    pub fn shifted(&self, step: i32) -> Result<Self, String> {
        let d = parse_date(&self.anchor)?;
        let next = match self.mode {
            Mode::Month => {
                let months = d.year() * 12 + d.month0() as i32 + step;
                let year = months.div_euclid(12);
                let month = months.rem_euclid(12) as u32 + 1;
                let mut day = d.day();
                loop {
                    if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
                        break d;
                    }
                    if day == 1 {
                        return Err("Mês inválido".into());
                    }
                    day -= 1
                }
            }
            Mode::Week => d + Duration::days(i64::from(step) * 7),
            Mode::Day => d + Duration::days(i64::from(step)),
        };
        let p = Self {
            mode: self.mode,
            anchor: next.to_string(),
        };
        p.validate()?;
        Ok(p)
    }
    pub fn month(&self) -> String {
        self.anchor.chars().take(7).collect()
    }
    pub fn calendar(&self) -> Result<Vec<CalendarDay>, String> {
        let (a, b) = self.dates()?;
        let first = if self.mode == Mode::Month {
            a - Duration::days(i64::from(a.weekday().num_days_from_monday()))
        } else {
            a
        };
        let count = if self.mode == Mode::Month {
            ((b - first).num_days() + 6) / 7 * 7
        } else {
            (b - a).num_days()
        };
        Ok((0..count)
            .map(|n| {
                let d = first + Duration::days(n);
                CalendarDay {
                    date: d.to_string(),
                    number: d.day(),
                    in_period: d >= a && d < b,
                    weekday: d.weekday().num_days_from_monday(),
                }
            })
            .collect())
    }
}
#[derive(Clone, Debug)]
pub struct CalendarDay {
    pub date: String,
    pub number: u32,
    pub in_period: bool,
    pub weekday: u32,
}
fn midnight(date: NaiveDate) -> Result<i64, String> {
    // Some historical DST transitions skip midnight. Use the first real minute.
    for minute in 0..180 {
        let time = date
            .and_hms_opt(minute / 60, minute % 60, 0)
            .ok_or("Data inválida")?;
        if let Some(d) = Local.from_local_datetime(&time).earliest() {
            return Ok(d.timestamp_millis());
        }
    }
    Err("Início do dia indisponível no fuso local".into())
}
pub fn local_day(timestamp: &str) -> Result<(String, String, i64), String> {
    let d = chrono::DateTime::parse_from_rfc3339(timestamp)
        .or_else(|_| chrono::DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .map_err(|_| "O Jira retornou um apontamento com data inválida")?;
    let local = d.with_timezone(&Local);
    Ok((
        local.date_naive().to_string(),
        local.format("%H:%M").to_string(),
        d.timestamp_millis(),
    ))
}
/// A calendar draft uses the selected local day; UTC offset follows that day.
/// Ambiguous local times use the earlier occurrence; skipped times are explicit.
pub fn worklog_start(date: &str) -> Result<String, String> {
    let now = Local::now();
    let date = parse_date(date)?;
    let time = date.and_time(now.time());
    Local
        .from_local_datetime(&time)
        .earliest()
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .ok_or_else(|| "Horário inexistente no fuso local; ajuste o início do apontamento".into())
}
pub fn month_name(n: u32) -> &'static str {
    match n {
        1 => "Janeiro",
        2 => "Fevereiro",
        3 => "Março",
        4 => "Abril",
        5 => "Maio",
        6 => "Junho",
        7 => "Julho",
        8 => "Agosto",
        9 => "Setembro",
        10 => "Outubro",
        11 => "Novembro",
        12 => "Dezembro",
        _ => "Mês",
    }
}
pub fn duration(seconds: i64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if s != 0 {
        format!("{h}h {m:02}min {s:02}s")
    } else if m != 0 {
        format!("{h}h {m:02}min")
    } else {
        format!("{h}h")
    }
}
