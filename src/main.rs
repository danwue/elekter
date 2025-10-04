use chrono_tz::Tz;
use itertools::Itertools;
use reqwest::blocking::Client;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime};
use structopt::{
    StructOpt,
    clap::{crate_authors, crate_description, crate_name},
};

use validator::{Validate, ValidationError};

use chrono::{DateTime, Datelike, Days, NaiveTime, Timelike, Utc};
use chrono_tz::Europe::Tallinn;
use nonempty::NonEmpty;
use ordered_float::NotNan;
use serde::Deserialize;

fn must_be_true(v: &bool) -> Result<(), ValidationError> {
    if *v {
        Ok(())
    } else {
        Err(ValidationError::new("must_be_true"))
    }
}

fn validate_constraints(v: &Device) -> Result<(), ValidationError> {
    if v.window.is_some() && (v.ratio_min.is_none() && v.ratio_max.is_none()) {
        Err(ValidationError::new(
            "Window can only be specified if either ratio_min or ratio_max is specified",
        ))
    } else if v.threshold.is_none() && v.ratio_max.is_some() {
        Err(ValidationError::new(
            "Threshold is needed i ratio_max is specified",
        ))
    } else if let Some(ratio_min) = v.ratio_min && let Some(ratio_max) = v.ratio_max && ratio_min > ratio_max {
        Err(ValidationError::new(
            "ratio_max must be bigger than ratio_min",
        ))
    } else {
        Ok(())
    }
}

#[derive(StructOpt)]
#[structopt(author = crate_authors!(), name = crate_name!(), about = crate_description!())]
struct Opt {
    /// Simulate current day, without executing any commands
    #[structopt(short = "n", long)]
    dry_run: bool,

    /// TOML configuration file
    #[structopt(parse(from_os_str))]
    config: PathBuf,
}

#[derive(Deserialize, Validate)]
struct Conf {
    package: Package,
    #[serde(flatten)]
    #[validate(nested)]
    devices: BTreeMap<String, Device>,
}

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[validate(schema(function = "validate_constraints"))]
struct Device {
    threshold: Option<NotNan<f32>>,
    #[validate(range(min = 0.0, max = 1.0))]
    ratio_min: Option<NotNan<f32>>,
    #[validate(range(min = 0.0, max = 1.0))]
    ratio_max: Option<NotNan<f32>>,
    #[serde(default, with = "humantime_serde::option")]
    window: Option<Duration>,
    cmd_on: NonEmpty<String>,
    cmd_off: NonEmpty<String>,
}

#[derive(Deserialize, Validate)]
struct PriceResponse {
    #[validate(custom(function = "must_be_true", message = "success must be true"))]
    success: bool,
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    ee: NonEmpty<Price>,
}
#[derive(Deserialize)]
struct Price {
    #[serde(with = "chrono::serde::ts_seconds")]
    timestamp: DateTime<Utc>,
    price: NotNan<f32>,
}
#[derive(Deserialize)]
struct Package {
    day: NotNan<f32>,
    night: NotNan<f32>,
}

fn load_config(file: &PathBuf) -> Result<Conf, Box<dyn std::error::Error>> {
    Ok(toml::from_str(&std::fs::read_to_string(file)?)?)
}

fn fetch_prices(
    start: &DateTime<Tz>,
    end: &DateTime<Tz>,
) -> Result<NonEmpty<Price>, Box<dyn std::error::Error>> {
    let params = [("start", start.to_rfc3339()), ("end", end.to_rfc3339())];
    let prices: PriceResponse = Client::new()
        .get("https://dashboard.elering.ee/api/nps/price")
        .query(&params)
        .send()?
        .json()?;
    prices.validate()?;
    Ok(prices.data.ee)
}

fn add_grid_rate(price: &Price, package: &Package) -> Price {
    let local_time = price.timestamp.with_timezone(&Tallinn);
    let local_time_hour = local_time.hour();
    let current_grid =
        if (7..22).contains(&local_time_hour) && local_time.weekday().number_from_monday() < 6 {
            package.day
        } else {
            package.night
        };
    Price {
        price: price.price + current_grid,
        ..*price
    }
}

fn satisfy_constraints(prices: &NonEmpty<Price>, device: &Device) -> BTreeSet<DateTime<Utc>> {
    let mut enabled = BTreeSet::new();
    if let Some(threshold) = device.threshold {
        prices
            .iter()
            .filter(|p| p.price <= threshold)
            .for_each(|p| {
                enabled.insert(p.timestamp);
            });
        if let Some(ratio_max) = device.ratio_max {
            let interval = (prices.last().timestamp.timestamp()
                - prices.first().timestamp.timestamp()) as usize
                / (prices.len() - 1);
            let window_size = device
                .window
                .map(|dur| dur.as_secs() as usize / interval)
                .unwrap_or(prices.len());
            let max_enable_per_window = (ratio_max * window_size as f32).floor() as usize;
            for window in prices.iter().collect_vec().windows(window_size) {
                window
                    .iter()
                    .sorted_by_key(|p| p.price)
                    .skip(max_enable_per_window)
                    .for_each(|p| {
                        enabled.remove(&p.timestamp);
                    });
            }
        }
    }
    if let Some(ratio_min) = device.ratio_min {
        let interval = (prices.last().timestamp.timestamp() - prices.first().timestamp.timestamp())
            as usize
            / (prices.len() - 1);
        let window_size = device
            .window
            .map(|dur| dur.as_secs() as usize / interval)
            .unwrap_or(prices.len());
        let enable_per_window = (ratio_min * window_size as f32).ceil() as usize;
        for window in prices.iter().collect_vec().windows(window_size) {
            window
                .iter()
                .sorted_by_key(|p| p.price)
                .take(enable_per_window)
                .for_each(|p| {
                    enabled.insert(p.timestamp);
                });
        }
    }
    enabled
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // parse command line arguments and config file
    let opt = Opt::from_args();
    let conf = load_config(&opt.config)?;
    conf.validate()?;

    for day in std::iter::successors(
        Some(
            Utc::now()
                .with_timezone(&Tallinn)
                .with_time(NaiveTime::MIN)
                .unwrap(),
        ),
        |d| Some(d.checked_add_days(Days::new(1)).unwrap()),
    ) {
        // load market prices for one full day
        let market_prices = fetch_prices(
            &day,
            &day.with_time(
                NaiveTime::from_num_seconds_from_midnight_opt(24 * 60 * 60 - 1, 0).unwrap(),
            )
            .unwrap(),
        )?;

        // adjust market prices with day/night rates based on selected network package
        let consumer_prices = market_prices.map(|p| add_grid_rate(&p, &conf.package));

        // calculate enabled times for devices based on constraints
        let thresholds: BTreeMap<&String, (&Device, BTreeSet<DateTime<Utc>>)> = conf
            .devices
            .iter()
            .map(|(name, constraints)| {
                (
                    name,
                    (
                        constraints,
                        satisfy_constraints(&consumer_prices, constraints),
                    ),
                )
            })
            .collect();

        for price in consumer_prices {
            if !opt.dry_run {
                let sys_time: SystemTime = price.timestamp.into();
                if let Ok(dur) = sys_time.duration_since(SystemTime::now()) {
                    thread::sleep(dur);
                } else {
                    continue; // time in the past, no need to process
                }
            }
            println!(
                "{} ({:6.2} EUR/MWh)",
                price.timestamp.with_timezone(&Tallinn).naive_local(),
                price.price
            );
            for (dev_name, (dev_conf, en_times)) in &thresholds {
                let status = en_times.contains(&price.timestamp);
                println!(
                    "  {dev_name}: {}",
                    if status { "enabled" } else { "disabled" }
                );
                if !opt.dry_run {
                    let (cmd, args) = if status {
                        dev_conf.cmd_on.split_first()
                    } else {
                        dev_conf.cmd_off.split_first()
                    };
                    let output = Command::new(cmd).args(args).output()?;
                    if let Some(exit_code) = output.status.code() {
                        println!("    {cmd} {} (exit {exit_code})", args.join(" "));
                    }
                }
            }
        }

        if opt.dry_run {
            break;
        }
    }
    Ok(())
}
