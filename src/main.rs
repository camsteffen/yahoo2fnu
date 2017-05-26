extern crate chrono;
#[macro_use]
extern crate error_chain;
extern crate hyper;
extern crate serde_json;

use std::result::Result as StdResult;
use chrono::NaiveDate;
use chrono::UTC;
use hyper::Url;
use hyper::Client;
use hyper::client::Response;
use hyper::header::Cookie;
use hyper::header::SetCookie;
use hyper::status::StatusCode;
use std::env::temp_dir;
use std::fs::File;
use std::fs::remove_file;
use std::io::Read;
use std::io::Write;
use std::io::stdin;
use std::io::stdout;
use std::io;
use std::path::Path;
use std::path::PathBuf;

mod errors {
    error_chain! {
        errors {
            ParseCrumb {
                description("unable to parse crumb")
            }
            UnexpectedStatusCode(status_code: ::StatusCode) {
                description("unexpected response")
                display("unexpected response: : {}", status_code)
            }
        }

        foreign_links {
            Io(::io::Error);
            Network(::hyper::Error);
        }
    }
}

use errors::*;

fn first_char_capital(input: &str) -> char {
    input.chars().nth(0).unwrap()
        .to_uppercase().next().unwrap()
}

trait PromptOption: Sized + 'static {

    fn from_char(c: char) -> Option<Self>;

    fn prompt_str() -> &'static str;

    fn options() -> &'static [Self];

    fn option_str(&self) -> &'static str;

    fn prompt() -> Self {
        println!();
        for data_value in Self::options() {
            println!(" {}", data_value.option_str());
        }
        println!();
        prompt(Self::prompt_str(), None, |input| {
            let c = first_char_capital(input);
            match Self::from_char(c) {
                Some(val) => Ok(val),
                None => Err("Invalid choice".to_string()),
            }
        })
    }
}

enum DataValue { High, Low, Open, Close, AdjustedClose, Volume }

impl DataValue {
    fn col_name(&self) -> &str {
        match *self {
            DataValue::High => "High",
            DataValue::Low => "Low",
            DataValue::Open => "Open",
            DataValue::Close => "Close",
            DataValue::AdjustedClose => "Adj Close",
            DataValue::Volume => "Volume",
        }
    }
}

impl PromptOption for DataValue {
    fn from_char(c: char) -> Option<DataValue> {
        let data_value = match c {
            'H' => DataValue::High,
            'L' => DataValue::Low,
            'O' => DataValue::Open,
            'C' => DataValue::Close,
            'A' => DataValue::AdjustedClose,
            'V' => DataValue::Volume,
            _ => return None,
        };
        Some(data_value)
    }

    fn prompt_str() -> &'static str {
        "Data Value"
    }

    fn options() -> &'static [DataValue] {
        static OPTIONS: [DataValue; 6] = [
            DataValue::High,
            DataValue::Low,
            DataValue::Open,
            DataValue::Close,
            DataValue::AdjustedClose,
            DataValue::Volume,
        ];
        &OPTIONS
    }

    fn option_str(&self) -> &'static str {
        match *self {
            DataValue::High => "(H)igh",
            DataValue::Low => "(L)ow",
            DataValue::Open => "(O)pen",
            DataValue::Close => "(C)lose",
            DataValue::AdjustedClose => "(A)djusted Close",
            DataValue::Volume => "(V)olume",
        }
    }
}

enum Interval { Daily, Weekly, Monthly }

impl Interval {
    fn param_val(&self) -> &str {
        match *self {
            Interval::Daily => "1d",
            Interval::Weekly => "1wk",
            Interval::Monthly => "1mo",
        }
    }
}

impl PromptOption for Interval {
    fn options() -> &'static [Interval] {
        static OPTIONS: [Interval; 3] = [
            Interval::Daily,
            Interval::Weekly,
            Interval::Monthly,
        ];
        &OPTIONS
    }

    fn from_char(c: char) -> Option<Interval> {
        let data_value = match c {
            'D' => Interval::Daily,
            'W' => Interval::Weekly,
            'M' => Interval::Monthly,
            _ => return None,
        };
        Some(data_value)
    }

    fn prompt_str() -> &'static str {
        "Interval"
    }

    fn option_str(&self) -> &'static str {
        match *self {
            Interval::Daily => "(D)aily",
            Interval::Weekly => "(W)eekly",
            Interval::Monthly => "(M)onthly",
        }
    }
}

struct CookieData { cookie: String, crumb: String }

fn main() {
    run().unwrap_or_else(|err| {
        print!("Error");
        for err in err.iter() {
            print!(": {}", err);
        }
        println!();
    });
}

fn confirm_replace(path: &Path) -> bool {
    let replace_prompt = format!("{} already exists. Replace? [Y/n]", path.display());
    prompt(&replace_prompt, Some(true), |input| {
        match first_char_capital(input) {
            'Y' => Ok(true),
            'N' => Ok(false),
            _ => Err("Invalid choice".to_string()),
        }
    })
}

fn run() -> Result<()> {
    let symbol = prompt_symbol();
    let data_value = DataValue::prompt();
    let start_date = prompt_date("Start Date", "min", 0);
    let end_date = prompt_date("End Date", "max", UTC::now().timestamp());
    let interval = Interval::prompt();
    let mut file = prompt_file(&symbol);
    let client = Client::new();
    let cookie_path = cookie_path();
    let cookie_data = get_cookie(&cookie_path, &client)?;
    let csv = fetch_csv(&client, &symbol, cookie_data, start_date, end_date, &interval).map_err(|err| {
        match delete_cookie(&cookie_path) {
            Ok(()) => err,
            Err(err2) => Error::with_chain(err, Error::from(err2)),
        }
    })?;
    write_fnu(&mut file, &csv, &symbol, &data_value)
}

fn prompt<T, F>(prompt: &str, default: Option<T>, f: F) -> T
        where F: Fn(&str) -> StdResult<T, String> {
    let mut buf = String::with_capacity(20);
    loop {
        buf.clear();
        print!("{}: ", prompt);
        stdout().flush().unwrap();
        stdin().read_line(&mut buf).unwrap();

        let input = buf.trim_right();
        if input.is_empty() {
            if let Some(val) = default {
                return val
            }
        } else {
            match f(buf.trim_right()) {
                Ok(val) => return val,
                Err(msg) => {
                    println!("{}", msg);
                    if !msg.is_empty() { println!() }
                },
            }
        }
    }
}

fn prompt_symbol() -> String {
    prompt("Symbol", None, |input| Ok(input.to_uppercase()))
}

fn prompt_date(name: &str, default_str: &str, default: i64) -> i64 {
    let prompt_str = format!("{} [mm-dd-yyyy] ({})", name, default_str);
    prompt(&prompt_str, Some(default), |input| {
        let date = match NaiveDate::parse_from_str(input, "%m-%d-%Y") {
            Ok(date) => date,
            Err(err) => return Err(format!("Invalid date: {}", err)),
        };
        Ok(date.and_hms(0, 0, 0).timestamp())
    })
}

fn prompt_file(symbol: &str) -> File {
    loop {
        let save_path = prompt_save_path(symbol);
        if save_path.exists() && !confirm_replace(&save_path) { continue }
        return match File::create(&save_path) {
            Ok(file) => file,
            Err(err) => {
                println!("Failed to create {}: {}", save_path.display(), err);
                continue
            },
        };
    }
}

fn default_path(symbol: &str) -> PathBuf {
    let file = PathBuf::from(format!("{}.fnu", symbol));
    if cfg!(target_os = "windows") {
        let mut path = PathBuf::from("C:\\FT");
        if path.exists() {
            path.push(file);
            return path
        }
    }
    file
}

fn prompt_save_path(symbol: &str) -> PathBuf {
    let default = default_path(symbol);
    let prompt_str = format!("Save To ({})", default.display());
    prompt(&prompt_str, Some(default), |input| {
        let mut path = PathBuf::from(input);
        if path.extension().is_none() {
            path.set_extension("fnu");
        }
        Ok(path)
    })
}

fn cookie_path() -> PathBuf {
    let mut path = temp_dir();
    path.push("yahoo2fnu_cookie.txt");
    path
}

fn get_cookie(path: &Path, client: &Client) -> Result<CookieData> {
    let cached = path.exists();
    let cookie_data = if cached {
        get_cookie_from_file(path)?
    } else {
        let cookie_data = get_cookie_from_web(client)?;
        save_cookie_file(path, &cookie_data)?;
        cookie_data
    };
    Ok(cookie_data)
}

fn delete_cookie(path: &Path) -> Result<()> {
    println!("Deleting cached cookie");
    remove_file(path)?;
    Ok(())
}

fn get_cookie_from_file(path: &Path) -> Result<CookieData> {
    println!("Using cached cookie");

    let mut file = File::open(&path)?;
    let mut buf = String::with_capacity(200);
    file.read_to_string(&mut buf)?;
    let parts = buf.lines().collect::<Vec<_>>();
    if parts.len() != 2 {
        delete_cookie(path)?;
        bail!("wrong number of lines");
    }
    let cookie_data = CookieData {
        cookie: parts[0].to_string(),
        crumb: parts[1].to_string(),
    };
    Ok(cookie_data)
}

fn get_cookie_from_web(client: &Client) -> Result<CookieData> {
    println!("Fetching cookie");

    let url = "http://finance.yahoo.com/quote/^GSPC";
    let mut res = client.get(url).send()?;
    ensure!(res.status == hyper::Ok, ErrorKind::UnexpectedStatusCode(res.status));

    let cookie = get_cookie_from_response(&res)?;

    let mut buf = String::with_capacity(1000000);
    res.read_to_string(&mut buf)?;

    let crumb = scrape_crumb(&buf)?.to_string();

    let cookie_data = CookieData {
        cookie: cookie,
        crumb: crumb,
    };

    Ok(cookie_data)
}

fn get_cookie_from_response(res: &Response) -> Result<String> {
    let set_cookie = res.headers.get::<SetCookie>().ok_or("set-cookie header is missing")?;
    let part = set_cookie.iter().find(|s| s[..2] == *"B=").ok_or("failed to parse set-cookie header")?;
    let cookie_string = part.to_string();
    Ok(cookie_string)
}

fn scrape_crumb(body: &str) -> Result<String> {
    let mut i = body.find("\"CrumbStore\"").ok_or_else(|| ErrorKind::ParseCrumb)?;
    let mut part = &body[i + 14..];
    i = part.find("\"crumb\"").ok_or_else(|| ErrorKind::ParseCrumb)?;
    part = &part[i + 8..];
    i = part.find('"').ok_or_else(|| ErrorKind::ParseCrumb)?;
    part = &part[i..];
    i = part[1..].find('"').ok_or_else(|| ErrorKind::ParseCrumb)?;
    part = &part[..i + 2];

    let crumb = serde_json::from_str(part).chain_err(|| ErrorKind::ParseCrumb)?;
    Ok(crumb)
}

fn save_cookie_file(path: &Path, cookie_data: &CookieData) -> Result<()> {
    let mut file = File::create(path)?;
    write!(file, "{}\n{}", cookie_data.cookie, cookie_data.crumb)?;
    println!("Saved cookie");
    Ok(())
}

fn fetch_csv(client: &Client, symbol: &str, cookie_data: CookieData,
             start_date: i64, end_date: i64, interval: &Interval) -> Result<String> {
    println!("Fetching CSV");

    let start_date = start_date.to_string();
    let end_date = end_date.to_string();

    let params = [
        ("period1", start_date.as_str()),
        ("period2", &end_date),
        ("interval", interval.param_val()),
        ("events", "history"),
        ("crumb", &cookie_data.crumb),
    ];
    let url = &format!("http://query1.finance.yahoo.com/v7/finance/download/{}", symbol);
    let url = Url::parse_with_params(url, &params).chain_err(|| "failed to parse url")?;
    let mut res = client.get(url)
        .header(Cookie(vec![cookie_data.cookie]))
        .send()?;

    ensure!(res.status == hyper::Ok, ErrorKind::UnexpectedStatusCode(res.status));

    let mut buf = String::with_capacity(500000);
    res.read_to_string(&mut buf)?;
    Ok(buf)
}

fn write_fnu(file: &mut File, csv: &str, symbol: &str, data_value: &DataValue) -> Result<()> {
    println!("Saving FNU file");

    writeln!(file, "{symbol}\n{name}",
             symbol = symbol,
             name = format!("{} {}", symbol, data_value.col_name()))?;

    let mut lines = csv.lines();
    let column_headers = lines.next().ok_or("empty data")?;
    let col_name = data_value.col_name();
    let col = column_headers.split(',').position(|s| s == col_name).ok_or("expected \"Close\"")?;
    for line in lines {
        let mut parts = line.split(',');
        let date = parts.next().ok_or("not enough columns")?;
        let year = &date[0..4];
        let month = &date[5..7];
        let day = &date[8..10];
        let value = parts.nth(col - 1).ok_or("not enough columns")?;
        writeln!(file, "{month}/{day}/{year},{value},0",
                 year = year,
                 month = month,
                 day = day,
                 value = value)?;
    }

    Ok(())
}

