#[macro_use]
extern crate error_chain;
extern crate hyper;
extern crate serde_json;

use hyper::Url;
use hyper::client::Client;
use hyper::client::Response;
use hyper::header::Cookie;
use hyper::header::SetCookie;
use hyper::status::StatusCode;
use std::env::temp_dir;
use std::env;
use std::fs::File;
use std::fs::remove_file;
use std::io::Read;
use std::io::Write;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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

struct CookieData {
    cookie: String,
    crumb: String,
}

fn main() {
    run().unwrap_or_else(|err| {
        let mut trace = err.iter();
        eprint!("Error: {}", trace.next().unwrap());
        for err in trace {
            eprint!(":\n    {}", err);
        }
        eprintln!();
    });
}

fn run() -> Result<()> {
    let symbol = get_symbol()?;
    let client = Client::new();
    let cookie_path = cookie_path();
    let cookie_data = get_cookie(&cookie_path, &client)?;
    let csv = fetch_csv(&client, &symbol, cookie_data).map_err(|err| {
        match remove_file(&cookie_path) {
            Ok(()) => err,
            Err(err2) => Error::with_chain(err, Error::from(err2)),
        }
    })?;
    write_fnu(&symbol, &csv)
}

fn get_symbol() -> Result<String> {
    let args = env::args().collect::<Vec<_>>();
    let symbol = match args.len() {
        0 ... 1 => bail!("no symbol provided"),
        2 => &args[1],
        _ => bail!("too many arguments"),
    };
    Ok(symbol.to_uppercase())
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

fn get_cookie_from_file(path: &Path) -> Result<CookieData> {
    eprintln!("Using cached cookie");

    let mut file = File::open(&path)?;
    let mut buf = String::with_capacity(200);
    file.read_to_string(&mut buf)?;
    let parts = buf.lines().collect::<Vec<_>>();
    if parts.len() != 2 {
        remove_file(path)?;
        bail!("wrong number of lines");
    }
    let cookie_data = CookieData {
        cookie: parts[0].to_string(),
        crumb: parts[1].to_string(),
    };
    Ok(cookie_data)
}

fn get_cookie_from_web(client: &Client) -> Result<CookieData> {
    eprintln!("Fetching cookie");

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
    eprintln!("Saved cookie");
    Ok(())
}

fn fetch_csv(client: &Client, symbol: &str, cookie_data: CookieData) -> Result<String> {
    eprintln!("Fetching CSV");

    let now = SystemTime::now();
    let now = now.duration_since(UNIX_EPOCH).chain_err(|| "failed to calculate current time")?;

    let params = [
        ("period1", "345448800"), // 12/12/1980
        ("period2", &now.as_secs().to_string()),
        ("interval", "1d"),
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

fn write_fnu(symbol: &str, csv: &str) -> Result<()> {
    eprintln!("Writing FNU");

    println!("{symbol}\n{name}", symbol = symbol, name = symbol);

    let mut lines = csv.lines();
    let column_headers = lines.next().ok_or("empty data")?;
    let col = column_headers.split(',').position(|s| s == "Close").ok_or("expected \"Close\"")?;
    for line in lines {
        let mut parts = line.split(',');
        let date = parts.next().ok_or("not enough columns")?;
        let year = &date[0..4];
        let month = &date[5..7];
        let day = &date[8..10];
        let close = parts.nth(col - 1).ok_or("not enough columns")?;
        println!("{month}/{day}/{year},{close},0",
                 year = year,
                 month = month,
                 day = day,
                 close = close);
    }

    Ok(())
}

