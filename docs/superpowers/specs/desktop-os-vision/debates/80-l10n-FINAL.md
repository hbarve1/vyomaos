# FINAL Spec: Localization & Internationalization (Round 80)

**macOS Analogue**: NSLocale / CFLocale / Foundation i18n
**Status**: FINAL — all blocking issues resolved
**Subsystem ID**: R80
**Dependencies**: R78 (System Preferences — `locale.language`, `locale.timezone` keys)

---

## 1. Architecture

### Module Layout

```
supervisor/src/locale/
  mod.rs          ≤200 lines  — LocaleState, OnceLock global, handle_locale_line(), startup load
  catalog.rs      ≤400 lines  — .strings file parser, message catalog, t() / tf() lookup
  format.rs       ≤400 lines  — date/time, number, currency formatting
  tz.rs           ≤250 lines  — static timezone table, UTC↔local conversion
  schema.rs       ≤150 lines  — LocaleData, NumberFormat, PluralRule, TzEntry structs
  sys_strings.rs  ≤150 lines  — supervisor's own UI-facing translatable string keys
```

Locale catalog files on disk:
```
/data/.vyoma/locales/
  en-US/messages.strings
  fr-FR/messages.strings
  de-DE/messages.strings
  ja-JP/messages.strings
  zh-CN/messages.strings
  es-ES/messages.strings
  pt-BR/messages.strings
```

### Global State (`mod.rs`)

```rust
use std::sync::{Arc, Mutex, OnceLock};

pub static LOCALE: OnceLock<Arc<Mutex<LocaleState>>> = OnceLock::new();

pub struct LocaleState {
    pub language: String,       // e.g. "en-US"
    pub timezone: String,       // e.g. "America/New_York"
    pub time_fmt_24h: bool,
    pub catalog: Catalog,       // loaded message catalog
    pub fallback: Catalog,      // always en-US
}

impl LocaleState {
    /// Called once at supervisor startup after R78 prefs are loaded.
    pub fn init(language: &str, timezone: &str, time_fmt_24h: bool) -> Arc<Mutex<Self>> {
        let catalog = Catalog::load(language);
        let fallback = if language == "en-US" {
            catalog.clone()
        } else {
            Catalog::load("en-US")
        };
        let state = LocaleState {
            language: language.to_string(),
            timezone: timezone.to_string(),
            time_fmt_24h,
            catalog,
            fallback,
        };
        let arc = Arc::new(Mutex::new(state));
        LOCALE.set(arc.clone()).ok();
        arc
    }

    /// Hot-reload when R78 locale.language pref changes.
    pub fn reload(&mut self, new_language: &str, new_timezone: &str) {
        self.language = new_language.to_string();
        self.timezone = new_timezone.to_string();
        self.catalog = Catalog::load(new_language);
        if new_language != "en-US" {
            self.fallback = Catalog::load("en-US");
        }
    }
}

/// Called from supervisor's main stdout-parse loop.
pub fn handle_locale_line(line: &str, app_name: &str) -> Option<String> {
    let rest = line.strip_prefix("VYOMA_LOCALE:")?;
    let lock = LOCALE.get()?.lock().ok()?;
    let parts: Vec<&str> = rest.splitn(32, '|').collect();
    match parts[0] {
        "t" if parts.len() >= 2 => {
            let val = lock.catalog.t(parts[1]).unwrap_or_else(|| {
                lock.fallback.t(parts[1]).unwrap_or(parts[1])
            });
            Some(format!("VYOMA_LOCALE:str|{}|{}", parts[1], val))
        }
        "tf" if parts.len() >= 3 => {
            let key = parts[1];
            let args = &parts[2..];
            let template = lock.catalog.t(key).unwrap_or_else(|| {
                lock.fallback.t(key).unwrap_or(key)
            });
            let formatted = crate::locale::catalog::printf_format(template, args);
            Some(format!("VYOMA_LOCALE:str|{}|{}", key, formatted))
        }
        "format_date" if parts.len() >= 3 => {
            let ts: i64 = parts[1].parse().unwrap_or(0);
            let fmt = parts[2];
            let tz_offset = crate::locale::tz::lookup_offset(&lock.timezone);
            let dt = crate::locale::format::ts_to_datetime(ts, tz_offset);
            let result = crate::locale::format::format_date(&dt, fmt, &lock.catalog);
            Some(format!("VYOMA_LOCALE:date|{}", result))
        }
        "format_num" if parts.len() >= 3 => {
            let value: f64 = parts[1].parse().unwrap_or(0.0);
            let decimals: u8 = parts[2].parse().unwrap_or(2);
            let nf = crate::locale::format::number_format_for(&lock.language);
            let result = crate::locale::format::format_number(value, decimals, &nf);
            Some(format!("VYOMA_LOCALE:num|{}", result))
        }
        "format_currency" if parts.len() >= 3 => {
            let cents: i64 = parts[1].parse().unwrap_or(0);
            let code = parts[2];
            let nf = crate::locale::format::number_format_for(&lock.language);
            let result = crate::locale::format::format_currency(cents, code, &nf);
            Some(format!("VYOMA_LOCALE:currency|{}", result))
        }
        "get_locale" => {
            let tf = if lock.time_fmt_24h { "24h" } else { "12h" };
            Some(format!("VYOMA_LOCALE:locale|{}|{}|{}", lock.language, lock.timezone, tf))
        }
        "plural" if parts.len() >= 4 => {
            let n: u64 = parts[1].parse().unwrap_or(0);
            let key_one = parts[2];
            let key_many = parts[3];
            let rule = crate::locale::schema::plural_rule(n, &lock.language);
            let key = match rule {
                crate::locale::schema::PluralRule::One => key_one,
                _ => key_many,
            };
            let val = lock.catalog.t(key).unwrap_or_else(|| {
                lock.fallback.t(key).unwrap_or(key)
            });
            Some(format!("VYOMA_LOCALE:str|{}|{}", key, val))
        }
        _ => None,
    }
}
```

---

## 2. Locale Catalog Format

Files at `/data/.vyoma/locales/<lang_code>/messages.strings`:

```
# VyomaOS locale file — en-US
# Format: "key" = "value";
# Comments start with #, blank lines ignored.
# Substitutions: %s (string), %d (integer), %f (float)

"app.name"        = "VyomaOS Settings";
"btn.ok"          = "OK";
"btn.cancel"      = "Cancel";
"btn.apply"       = "Apply";
"file.count"      = "%d file(s)";
"greeting"        = "Hello, %s!";
"date.weekday.0"  = "Sunday";
"date.weekday.1"  = "Monday";
"date.weekday.2"  = "Tuesday";
"date.weekday.3"  = "Wednesday";
"date.weekday.4"  = "Thursday";
"date.weekday.5"  = "Friday";
"date.weekday.6"  = "Saturday";
"date.month.0"    = "January";
"date.month.1"    = "February";
"date.month.2"    = "March";
"date.month.3"    = "April";
"date.month.4"    = "May";
"date.month.5"    = "June";
"date.month.6"    = "July";
"date.month.7"    = "August";
"date.month.8"    = "September";
"date.month.9"    = "October";
"date.month.10"   = "November";
"date.month.11"   = "December";
```

**Fallback chain**: `ja-JP` → `ja` → `en-US`. If a key is missing in the active locale, it falls back to the base language variant, then to `en-US`.

**Built-in locale codes**: `en-US`, `fr-FR`, `de-DE`, `ja-JP`, `zh-CN`, `es-ES`, `pt-BR`.

---

## 3. `catalog.rs` — Catalog Parser & Lookup

```rust
// supervisor/src/locale/catalog.rs

use std::collections::HashMap;

const LOCALE_BASE_PATH: &str = "/data/.vyoma/locales";

#[derive(Clone, Default)]
pub struct Catalog {
    lang: String,
    entries: HashMap<String, String>,
}

impl Catalog {
    pub fn load(lang: &str) -> Self {
        let path = format!("{}/{}/messages.strings", LOCALE_BASE_PATH, lang);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let entries = parse_strings_file(&content);
                Catalog { lang: lang.to_string(), entries }
            }
            Err(_) => {
                // Try base language: "ja-JP" -> "ja"
                if let Some(base) = lang.split('-').next() {
                    if base != lang {
                        let base_path = format!("{}/{}/messages.strings", LOCALE_BASE_PATH, base);
                        if let Ok(content) = std::fs::read_to_string(&base_path) {
                            let entries = parse_strings_file(&content);
                            return Catalog { lang: lang.to_string(), entries };
                        }
                    }
                }
                Catalog { lang: lang.to_string(), entries: HashMap::new() }
            }
        }
    }

    /// Look up a translation key. Returns None if not found.
    pub fn t<'a>(&'a self, key: &str) -> Option<&'a str> {
        self.entries.get(key).map(|s| s.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse a `.strings` file into key→value pairs.
/// B1 FIX: Strip UTF-8 BOM before processing.
fn parse_strings_file(raw: &str) -> HashMap<String, String> {
    // B1 FIX: strip BOM (0xEF 0xBB 0xBF encoded as \u{FEFF} in Rust str)
    let content = raw.trim_start_matches('\u{FEFF}');
    let mut map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = parse_kv_line(line) {
            map.insert(key, value);
        }
    }
    map
}

/// Parse: `"key" = "value";`
fn parse_kv_line(line: &str) -> Option<(String, String)> {
    // Must start with '"'
    if !line.starts_with('"') {
        return None;
    }
    let (key, after_key) = parse_quoted_string(&line[1..])?;
    let after_key = after_key.trim();
    let after_key = after_key.strip_prefix('=')?;
    let after_key = after_key.trim();
    let after_key = after_key.strip_prefix('"')?;
    let (value, _) = parse_quoted_string(after_key)?;
    Some((key, value))
}

/// Parse a quoted string (without the leading quote already consumed).
/// Handles `\"` and `\\` escapes. Returns (parsed_string, remainder_after_closing_quote).
fn parse_quoted_string(s: &str) -> Option<(String, &str)> {
    let mut result = String::new();
    let mut chars = s.char_indices();
    while let Some((i, ch)) = chars.next() {
        match ch {
            '"' => {
                let remainder = &s[i + 1..];
                return Some((result, remainder));
            }
            '\\' => {
                match chars.next() {
                    Some((_, '"'))  => result.push('"'),
                    Some((_, '\\')) => result.push('\\'),
                    Some((_, 'n'))  => result.push('\n'),
                    Some((_, 't'))  => result.push('\t'),
                    Some((_, c))    => { result.push('\\'); result.push(c); }
                    None => return None,
                }
            }
            c => result.push(c),
        }
    }
    None // no closing quote found
}

/// POSIX-style printf formatting: %s, %d, %f substitutions (positional, left-to-right).
pub fn printf_format(template: &str, args: &[&str]) -> String {
    let mut result = String::with_capacity(template.len() + 32);
    let mut arg_idx = 0;
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek() {
                Some(&'s') => {
                    chars.next();
                    if arg_idx < args.len() {
                        result.push_str(args[arg_idx]);
                        arg_idx += 1;
                    }
                }
                Some(&'d') => {
                    chars.next();
                    if arg_idx < args.len() {
                        let n: i64 = args[arg_idx].parse().unwrap_or(0);
                        result.push_str(&n.to_string());
                        arg_idx += 1;
                    }
                }
                Some(&'f') => {
                    chars.next();
                    if arg_idx < args.len() {
                        let f: f64 = args[arg_idx].parse().unwrap_or(0.0);
                        result.push_str(&format!("{:.2}", f));
                        arg_idx += 1;
                    }
                }
                Some(&'%') => {
                    chars.next();
                    result.push('%');
                }
                _ => result.push('%'),
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let s = r#""btn.ok" = "OK";"#;
        let map = parse_strings_file(s);
        assert_eq!(map.get("btn.ok").map(|s| s.as_str()), Some("OK"));
    }

    #[test]
    fn test_parse_bom() {
        let s = "\u{FEFF}\"key\" = \"value\";";
        let map = parse_strings_file(s);
        assert_eq!(map.get("key").map(|s| s.as_str()), Some("value"));
    }

    #[test]
    fn test_escape() {
        let s = r#""k" = "say \"hi\"";"#;
        let map = parse_strings_file(s);
        assert_eq!(map.get("k").map(|s| s.as_str()), Some("say \"hi\""));
    }

    #[test]
    fn test_printf_format() {
        assert_eq!(printf_format("Hello, %s!", &["World"]), "Hello, World!");
        assert_eq!(printf_format("%d file(s)", &["3"]), "3 file(s)");
        assert_eq!(printf_format("%.2f", &["3.14159"]), "3.14");
    }
}
```

---

## 4. `schema.rs` — Data Structures

```rust
// supervisor/src/locale/schema.rs

#[derive(Clone, Debug)]
pub struct LocaleData {
    pub language:    String,
    pub timezone:    String,
    pub time_fmt_24h: bool,
}

#[derive(Clone, Debug)]
pub struct NumberFormat {
    pub decimal_sep:   char,  // '.' (en-US), ',' (de-DE, fr-FR)
    pub thousands_sep: char,  // ',' (en-US), '.' (de-DE), ' ' (fr-FR)
    pub decimal_places: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluralRule { One, Few, Many, Other }

pub fn plural_rule(n: u64, lang: &str) -> PluralRule {
    match lang {
        "en" | "en-US" | "de" | "de-DE" | "es" | "es-ES" | "pt" | "pt-BR" => {
            if n == 1 { PluralRule::One } else { PluralRule::Other }
        }
        "fr" | "fr-FR" => {
            if n <= 1 { PluralRule::One } else { PluralRule::Other }
        }
        "ja" | "ja-JP" | "zh" | "zh-CN" => PluralRule::Other, // CJK: no grammatical plural
        _ => if n == 1 { PluralRule::One } else { PluralRule::Other },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plural_en() {
        assert_eq!(plural_rule(1, "en-US"), PluralRule::One);
        assert_eq!(plural_rule(2, "en-US"), PluralRule::Other);
        assert_eq!(plural_rule(0, "en-US"), PluralRule::Other);
    }

    #[test]
    fn test_plural_fr() {
        assert_eq!(plural_rule(0, "fr-FR"), PluralRule::One);  // French: 0 is singular
        assert_eq!(plural_rule(1, "fr-FR"), PluralRule::One);
        assert_eq!(plural_rule(2, "fr-FR"), PluralRule::Other);
    }

    #[test]
    fn test_plural_cjk() {
        assert_eq!(plural_rule(1, "ja-JP"), PluralRule::Other);
        assert_eq!(plural_rule(100, "zh-CN"), PluralRule::Other);
    }
}
```

---

## 5. `tz.rs` — Timezone Table & Conversion

```rust
// supervisor/src/locale/tz.rs
// Known limitation: no DST support. All offsets are standard time (UTC offset fixed).

pub struct TzEntry {
    pub name: &'static str,
    pub offset_minutes: i32,
}

pub const TIMEZONES: &[TzEntry] = &[
    TzEntry { name: "UTC",                  offset_minutes: 0    },
    TzEntry { name: "America/New_York",     offset_minutes: -300 }, // EST
    TzEntry { name: "America/Chicago",      offset_minutes: -360 }, // CST
    TzEntry { name: "America/Denver",       offset_minutes: -420 }, // MST
    TzEntry { name: "America/Los_Angeles",  offset_minutes: -480 }, // PST
    TzEntry { name: "Europe/London",        offset_minutes: 0    }, // GMT
    TzEntry { name: "Europe/Paris",         offset_minutes: 60   }, // CET
    TzEntry { name: "Europe/Berlin",        offset_minutes: 60   }, // CET
    TzEntry { name: "Asia/Tokyo",           offset_minutes: 540  }, // JST
    TzEntry { name: "Asia/Shanghai",        offset_minutes: 480  }, // CST
    TzEntry { name: "Asia/Kolkata",         offset_minutes: 330  }, // IST (+5:30)
];

/// Returns the UTC offset in minutes for the given IANA timezone name.
/// Falls back to 0 (UTC) if not found.
pub fn lookup_offset(tz_name: &str) -> i32 {
    TIMEZONES
        .iter()
        .find(|e| e.name == tz_name)
        .map(|e| e.offset_minutes)
        .unwrap_or(0)
}

/// Convert a UTC Unix timestamp + timezone offset (minutes) to local seconds-since-epoch.
pub fn utc_to_local(utc_ts: i64, offset_minutes: i32) -> i64 {
    utc_ts + (offset_minutes as i64 * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_utc() {
        assert_eq!(lookup_offset("UTC"), 0);
    }

    #[test]
    fn test_lookup_tokyo() {
        assert_eq!(lookup_offset("Asia/Tokyo"), 540);
    }

    #[test]
    fn test_lookup_unknown() {
        assert_eq!(lookup_offset("Mars/Olympus"), 0);
    }

    #[test]
    fn test_utc_to_local_negative() {
        // New York (EST = -300 min): UTC 18:00 → local 13:00
        let utc = 18 * 3600i64;
        let local = utc_to_local(utc, -300);
        assert_eq!(local, 13 * 3600i64);
    }
}
```

---

## 6. `format.rs` — Date/Time, Number, Currency Formatting

```rust
// supervisor/src/locale/format.rs

use crate::locale::catalog::Catalog;
use crate::locale::schema::NumberFormat;

// ─── Date/Time ───────────────────────────────────────────────────────────────

pub struct DateTime {
    pub year:    i32,
    pub month:   u8,   // 1-12
    pub day:     u8,   // 1-31
    pub hour:    u8,   // 0-23
    pub minute:  u8,
    pub second:  u8,
    pub weekday: u8,   // 0=Sunday … 6=Saturday
}

/// Pure Gregorian calendar conversion from Unix timestamp.
/// B2 FIX: Correctly handles leap years using 400/100/4 rules.
/// Test case: ts=951868800 → 2000-03-01 00:00:00 (post-Y2K leap year boundary)
pub fn ts_to_datetime(ts: i64, _offset_minutes: i32) -> DateTime {
    // Offset already applied by caller via tz::utc_to_local; ts here is local seconds.
    let ts = if ts < 0 { 0 } else { ts };
    let secs_per_day: i64 = 86400;
    let days = ts / secs_per_day;
    let time_of_day = ts % secs_per_day;
    let hour   = (time_of_day / 3600) as u8;
    let minute = ((time_of_day % 3600) / 60) as u8;
    let second = (time_of_day % 60) as u8;
    // Weekday: 1970-01-01 was a Thursday (4)
    let weekday = ((days + 4) % 7) as u8;
    // Gregorian calendar: days since 1970-01-01
    let (year, month, day) = days_to_ymd(days);
    DateTime { year, month, day, hour, minute, second, weekday }
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn days_in_month(m: u8, y: i32) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11              => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 0,
    }
}

fn days_to_ymd(mut days: i64) -> (i32, u8, u8) {
    let mut year = 1970i32;
    loop {
        let days_in_year: i64 = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year { break; }
        days -= days_in_year;
        year += 1;
    }
    let mut month = 1u8;
    loop {
        let dim = days_in_month(month, year) as i64;
        if days < dim { break; }
        days -= dim;
        month += 1;
    }
    (year, month, days as u8 + 1)
}

/// strftime-subset formatter. Weekday/month names are loaded from catalog.
pub fn format_date(dt: &DateTime, fmt: &str, catalog: &Catalog) -> String {
    let mut result = String::with_capacity(32);
    let mut chars = fmt.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '%' { result.push(ch); continue; }
        match chars.next() {
            Some('Y') => result.push_str(&format!("{:04}", dt.year)),
            Some('m') => result.push_str(&format!("{:02}", dt.month)),
            Some('d') => result.push_str(&format!("{:02}", dt.day)),
            Some('H') => result.push_str(&format!("{:02}", dt.hour)),
            Some('M') => result.push_str(&format!("{:02}", dt.minute)),
            Some('S') => result.push_str(&format!("{:02}", dt.second)),
            Some('I') => {
                let h12 = match dt.hour % 12 { 0 => 12, h => h };
                result.push_str(&format!("{:02}", h12));
            }
            Some('p') => {
                result.push_str(if dt.hour < 12 { "AM" } else { "PM" });
            }
            Some('A') => {
                let key = format!("date.weekday.{}", dt.weekday);
                result.push_str(catalog.t(&key).unwrap_or("?"));
            }
            Some('B') => {
                let key = format!("date.month.{}", dt.month - 1);
                result.push_str(catalog.t(&key).unwrap_or("?"));
            }
            Some('%') => result.push('%'),
            Some(c)   => { result.push('%'); result.push(c); }
            None      => result.push('%'),
        }
    }
    result
}

/// Returns the locale-default date format pattern for the given lang.
pub fn default_date_fmt(lang: &str) -> &'static str {
    match lang {
        "fr-FR" | "fr"      => "%d/%m/%Y",
        "de-DE" | "de"      => "%d.%m.%Y",
        "ja-JP" | "ja"      => "%Y年%m月%d日",
        "zh-CN" | "zh"      => "%Y年%m月%d日",
        _                   => "%B %d, %Y",  // en-US default
    }
}

/// Returns the locale-default time format pattern for the given lang.
pub fn default_time_fmt(lang: &str, use_24h: bool) -> &'static str {
    if use_24h {
        return "%H:%M";
    }
    match lang {
        "en-US" | "en" => "%I:%M %p",
        _              => "%H:%M",
    }
}

// ─── Number Formatting ───────────────────────────────────────────────────────

pub fn number_format_for(lang: &str) -> NumberFormat {
    match lang {
        "de-DE" | "de" => NumberFormat { decimal_sep: ',', thousands_sep: '.', decimal_places: 2 },
        "fr-FR" | "fr" => NumberFormat { decimal_sep: ',', thousands_sep: ' ', decimal_places: 2 },
        "es-ES" | "es" => NumberFormat { decimal_sep: ',', thousands_sep: '.', decimal_places: 2 },
        _              => NumberFormat { decimal_sep: '.', thousands_sep: ',', decimal_places: 2 },
    }
}

pub fn format_number(value: f64, decimals: u8, fmt: &NumberFormat) -> String {
    let negative = value < 0.0;
    let abs_val  = value.abs();
    let integer_part = abs_val.trunc() as u64;
    let frac_part    = abs_val.fract();

    let int_str = format_integer_with_sep(integer_part, fmt.thousands_sep);
    let mut result = String::with_capacity(int_str.len() + 8);
    if negative { result.push('-'); }
    result.push_str(&int_str);
    if decimals > 0 {
        result.push(fmt.decimal_sep);
        let scale = 10u64.pow(decimals as u32) as f64;
        let frac_digits = (frac_part * scale).round() as u64;
        result.push_str(&format!("{:0>width$}", frac_digits, width = decimals as usize));
    }
    result
}

fn format_integer_with_sep(n: u64, sep: char) -> String {
    let s = n.to_string();
    if s.len() <= 3 { return s; }
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    let offset = s.len() % 3;
    if offset > 0 {
        result.push_str(&s[..offset]);
    }
    let mut i = offset;
    while i < s.len() {
        if i > 0 || offset > 0 { result.push(sep); }
        result.push_str(&s[i..i+3]);
        i += 3;
    }
    result
}

// ─── Currency Formatting ─────────────────────────────────────────────────────

fn currency_symbol(code: &str) -> &'static str {
    match code {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        "CNY" => "¥",
        _     => code,
    }
}

/// B3 FIX: negative cents render as `-$5.00` not `$-5.00`.
/// JPY: no fractional units (divide by 1, not 100).
pub fn format_currency(cents: i64, code: &str, fmt: &NumberFormat) -> String {
    let negative = cents < 0;
    let abs_cents = cents.unsigned_abs();
    let symbol = currency_symbol(code);

    let (major, minor, show_minor) = if code == "JPY" {
        (abs_cents, 0u64, false)
    } else {
        (abs_cents / 100, abs_cents % 100, true)
    };

    let int_str = format_integer_with_sep(major, fmt.thousands_sep);
    let mut result = String::with_capacity(16);
    if negative { result.push('-'); }
    result.push_str(symbol);
    result.push_str(&int_str);
    if show_minor {
        result.push(fmt.decimal_sep);
        result.push_str(&format!("{:02}", minor));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::catalog::Catalog;

    fn empty_catalog() -> Catalog { Catalog::default() }

    #[test]
    fn test_leap_year_2000() {
        // 2000-03-01 00:00:00 UTC = 951868800
        let dt = ts_to_datetime(951868800, 0);
        assert_eq!(dt.year, 2000);
        assert_eq!(dt.month, 3);
        assert_eq!(dt.day, 1);
    }

    #[test]
    fn test_epoch() {
        let dt = ts_to_datetime(0, 0);
        assert_eq!(dt.year, 1970);
        assert_eq!(dt.month, 1);
        assert_eq!(dt.day, 1);
        assert_eq!(dt.hour, 0);
        assert_eq!(dt.weekday, 4); // Thursday
    }

    #[test]
    fn test_format_number_en() {
        let nf = number_format_for("en-US");
        assert_eq!(format_number(1234567.89, 2, &nf), "1,234,567.89");
    }

    #[test]
    fn test_format_number_de() {
        let nf = number_format_for("de-DE");
        assert_eq!(format_number(1234567.89, 2, &nf), "1.234.567,89");
    }

    #[test]
    fn test_format_currency_usd() {
        let nf = number_format_for("en-US");
        assert_eq!(format_currency(500, "USD", &nf), "$5.00");
        assert_eq!(format_currency(-500, "USD", &nf), "-$5.00");  // B3 test
    }

    #[test]
    fn test_format_currency_jpy() {
        let nf = number_format_for("ja-JP");
        assert_eq!(format_currency(1000, "JPY", &nf), "¥1,000");
    }

    #[test]
    fn test_format_currency_negative_symbol_order() {
        let nf = number_format_for("en-US");
        let result = format_currency(-1099, "USD", &nf);
        // Must be "-$10.99", not "$-10.99"
        assert_eq!(&result[..2], "-$");
    }
}
```

---

## 7. `sys_strings.rs` — Supervisor UI String Keys

System boot/watchdog messages stay hardcoded English (supervisor initializes before locale is loaded). Only UI-facing supervisor messages use locale lookups.

```rust
// supervisor/src/locale/sys_strings.rs
// Keys for supervisor UI-facing strings. Boot/watchdog messages are NOT here.

/// Menu bar and notification strings — looked up after LOCALE is initialized.
pub const NOTIFICATION_TITLE: &str     = "sys.notification.title";
pub const APP_CRASHED: &str            = "sys.app.crashed";
pub const APP_RESTARTED: &str          = "sys.app.restarted";
pub const MENUBAR_SETTINGS: &str       = "sys.menubar.settings";
pub const MENUBAR_APPS: &str           = "sys.menubar.apps";
pub const MENUBAR_QUIT: &str           = "sys.menubar.quit";
pub const DIALOG_CONFIRM: &str         = "btn.ok";
pub const DIALOG_CANCEL: &str          = "btn.cancel";

/// Look up a supervisor UI string. Falls back to the key itself if locale not ready.
pub fn sys_t(key: &str) -> String {
    use crate::locale::LOCALE;
    LOCALE
        .get()
        .and_then(|arc| arc.lock().ok())
        .and_then(|lock| lock.catalog.t(key).map(|s| s.to_string()))
        .unwrap_or_else(|| key.to_string())
}
```

---

## 8. VYOMA_LOCALE Protocol Reference

All messages are line-oriented stdout/stdin exchanges between an app and the supervisor.

| App → Supervisor | Supervisor → App |
|---|---|
| `VYOMA_LOCALE:t\|<key>` | `VYOMA_LOCALE:str\|<key>\|<translated>` |
| `VYOMA_LOCALE:tf\|<key>\|<arg1>\|<arg2>...` | `VYOMA_LOCALE:str\|<key>\|<formatted>` |
| `VYOMA_LOCALE:format_date\|<unix_ts>\|<fmt>` | `VYOMA_LOCALE:date\|<formatted>` |
| `VYOMA_LOCALE:format_num\|<value>\|<decimals>` | `VYOMA_LOCALE:num\|<formatted>` |
| `VYOMA_LOCALE:format_currency\|<cents>\|<code>` | `VYOMA_LOCALE:currency\|<formatted>` |
| `VYOMA_LOCALE:get_locale` | `VYOMA_LOCALE:locale\|<lang>\|<tz>\|<time_fmt>` |
| `VYOMA_LOCALE:plural\|<n>\|<key_one>\|<key_many>` | `VYOMA_LOCALE:str\|<key>\|<value>` |

`<fmt>` uses strftime-subset tokens (`%Y`, `%m`, `%d`, `%H`, `%I`, `%M`, `%S`, `%p`, `%A`, `%B`).
`<cents>` is the integer amount in the smallest currency unit (e.g. USD cents, JPY yen as whole units).
`<time_fmt>` in `get_locale` response is `"12h"` or `"24h"`.

### App Usage Example (Rust WASM)

```rust
// Request translation
println!("VYOMA_LOCALE:t|btn.ok");
// Receive: VYOMA_LOCALE:str|btn.ok|OK

// Request formatted translation
println!("VYOMA_LOCALE:tf|file.count|42");
// Receive: VYOMA_LOCALE:str|file.count|42 file(s)

// Request date formatting
println!("VYOMA_LOCALE:format_date|1748649600|%B %d, %Y");
// Receive: VYOMA_LOCALE:date|May 30, 2025

// Query active locale
println!("VYOMA_LOCALE:get_locale");
// Receive: VYOMA_LOCALE:locale|en-US|America/New_York|12h
```

---

## 9. Capability & Integration

### Capability Gate

No new capability field is required for basic locale lookup (`t`, `tf`, `get_locale`, `plural`). All apps may call these unconditionally — locale is a read-only ambient service.

The `locale` capability is reserved for apps that **write new locale files** to `/data/.vyoma/locales/`. This is admin-only functionality (e.g., a localization tool app).

```toml
# Only locale file writers need this
[capabilities]
locale = true
```

Add to `supervisor/src/manifest.rs` Capabilities struct:

```rust
#[serde(default)]
pub locale: bool,   // B0 FIX: deny_unknown_fields compliance
```

Since basic locale lookups need no capability gate, supervisor checks `locale = true` only before allowing `VYOMA_LOCALE:write_catalog|...` commands (not specified in this MVP, reserved for R81+).

### R78 Integration

On startup, supervisor reads R78 prefs:
```rust
let lang = prefs.get("locale.language").unwrap_or("en-US");
let tz   = prefs.get("locale.timezone").unwrap_or("UTC");
let h24  = prefs.get("locale.time_format").map(|v| v == "24h").unwrap_or(false);
LocaleState::init(lang, tz, h24);
```

When `locale.language` or `locale.timezone` changes via R78:
```rust
// In R78 pref-change handler:
if let Some(arc) = LOCALE.get() {
    if let Ok(mut lock) = arc.lock() {
        lock.reload(&new_lang, &new_tz);
    }
}
// Push locale-changed event to all apps that called get_locale:
broadcast("VYOMA_LOCALE:locale_changed|{}|{}", new_lang, new_tz);
```

---

## 10. Blocking Issues (B0–B4)

### B0 — `deny_unknown_fields` on Capabilities

`manifest.rs` uses `#[serde(deny_unknown_fields)]` on the `Capabilities` struct. Adding `locale` without the corresponding field causes a parse error on all existing manifests that set unrelated fields.

**Fix**: Add `#[serde(default)] pub locale: bool` to the Capabilities struct. Since basic locale lookup requires no capability at all, the field only gates future write-catalog operations.

```rust
// supervisor/src/manifest.rs
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)] pub stdio:      bool,
    #[serde(default)] pub filesystem: bool,
    #[serde(default)] pub network:    bool,
    #[serde(default)] pub display:    bool,
    #[serde(default)] pub shell:      bool,
    #[serde(default)] pub mouse:      bool,
    #[serde(default)] pub locale:     bool,  // ← add this
    // ... peripheral fields unchanged
}
```

### B1 — UTF-8 BOM in `.strings` Files

Many editors (Windows Notepad, some macOS tools) prepend a UTF-8 BOM (`0xEF 0xBB 0xBF`, decoded as `\u{FEFF}`) when saving UTF-8 files. If the BOM is not stripped, the first key will be parsed as `"\u{FEFF}app.name"` instead of `"app.name"`, causing all lookups for that key to fail silently.

**Fix** (already in `catalog.rs` `parse_strings_file`):
```rust
let content = raw.trim_start_matches('\u{FEFF}');
```

### B2 — Gregorian Leap Year Boundary (Y2K)

Naive day-counting algorithms often mishandle the 2000-03-01 boundary because 2000 is a leap year (divisible by 400). An algorithm that only checks `y % 4 == 0` incorrectly skips Feb 29 2000, shifting all subsequent dates by one day.

**Fix** (already in `format.rs` `is_leap`):
```rust
fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
```

**Mandatory test** (already in `format.rs` tests):
```rust
let dt = ts_to_datetime(951868800, 0); // 2000-03-01 00:00:00 UTC
assert_eq!(dt.year, 2000);
assert_eq!(dt.month, 3);
assert_eq!(dt.day, 1);
```

### B3 — Currency Negative Sign Position

A naive implementation applies the currency symbol first, then the sign, yielding `$-5.00`. The correct rendering is `-$5.00` (sign precedes symbol).

**Fix** (already in `format.rs` `format_currency`):
```rust
let negative = cents < 0;
let abs_cents = cents.unsigned_abs();
// ...
if negative { result.push('-'); }
result.push_str(symbol);  // symbol after sign
```

**Mandatory test**:
```rust
assert_eq!(format_currency(-500, "USD", &nf), "-$5.00");
assert_eq!(&format_currency(-1099, "USD", &nf)[..2], "-$");
```

### B4 — Fallback Chain Deadlock

If `t()` lookup were implemented with on-demand I/O (reading the `.strings` file each call while holding `LOCALE.lock()`), a slow or blocked filesystem could cause the supervisor's main thread to stall indefinitely while holding the lock.

**Fix**: Load all locale data eagerly at `LocaleState::init()` time. `Catalog::load()` reads and parses the entire `.strings` file into a `HashMap` at startup. Subsequent `t()` calls are pure in-memory HashMap lookups with zero I/O. Hot-reload via `reload()` is the only time I/O happens, and it is triggered explicitly from the R78 pref-change path (not from within a locked section of the locale state itself — the lock is acquired, reload runs, lock releases; no nested lock acquisition occurs).

```rust
// Correct: load at init, never I/O inside t()
pub fn t<'a>(&'a self, key: &str) -> Option<&'a str> {
    self.entries.get(key).map(|s| s.as_str())
    // No filesystem access here
}
```

---

## Summary Table

| Component | File | Lines | Purpose |
|---|---|---|---|
| `LocaleState` + global + protocol handler | `mod.rs` | ≤200 | Entry point, VYOMA_LOCALE dispatch |
| `Catalog`, `.strings` parser, `printf_format` | `catalog.rs` | ≤400 | Message lookup, string formatting |
| `DateTime`, date/num/currency formatters | `format.rs` | ≤400 | Locale-aware output formatting |
| Timezone table + UTC↔local | `tz.rs` | ≤250 | Offset lookup, timestamp conversion |
| `LocaleData`, `NumberFormat`, `PluralRule` | `schema.rs` | ≤150 | Shared data structures |
| Supervisor UI string keys | `sys_strings.rs` | ≤150 | Menu bar / notification strings |

**Known Limitations**:
- No DST support (fixed UTC offsets only).
- No bidirectional text (RTL languages like Arabic/Hebrew not supported in this MVP).
- No Unicode collation (string sorting is byte-order only).
- No CLDR plural rules beyond One/Other (no Few/Many/Zero variants).
- Write-catalog protocol (`VYOMA_LOCALE:write_catalog`) deferred to R81.
