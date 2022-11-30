use aws_lambda_events::event::sns::SnsEvent;
use chrono::prelude::*;
use lambda_runtime::{handler_fn, Context, Error};
use log::LevelFilter;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use simple_logger::SimpleLogger;
use slack_hook3::{AttachmentBuilder, Field, Payload, PayloadBuilder, Slack};
use std::env;

#[tokio::main]
pub async fn main() -> Result<(), Error> {
    // Takes log level from RUST_LOG [off, error, warn, info, debug, trace]
    // https://docs.rs/env_logger/latest/env_logger/#enabling-logging
    SimpleLogger::new()
        .env()
        .with_level(LevelFilter::Info)
        .without_timestamps()
        .init()
        .unwrap();

    let handler = handler_fn(handler);

    lambda_runtime::run(handler).await?;

    Ok(()) // FIXME: This can never return an Error
}

/// Function entrypoint for the Lambda runtime
async fn handler(event: SnsEvent, _: Context) -> Result<Value, Error> {
    let message: Message = serde_json::from_str(&event.records[0].sns.message.as_ref().unwrap())
        .expect("Failed to deserialize message, wrong format.");

    let webhook =
        env::var("WEBHOOK_URL").expect("ERR: WEBHOOK_URL environment variable not set, fatal");

    log::debug!("WEBHOOK_URL={}", webhook);

    send(&webhook, message.build_payload()).await;

    Ok(json!({ "message": format!("OK") }))
}

async fn send(webhook: &str, p: Payload) {
    let slack = Slack::new(webhook).unwrap();
    let res = slack.send(&p).await;

    // Logs an error if Slack rejects the message or goes down but...
    // FIXME: Ensure Lambda returns 500 or panics to guarantee it hits the error metrics
    match res {
        Ok(()) => log::info!("Message sent to Slack"),
        Err(e) => log::error!("ERR: {:?}", e),
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    version: String,
    id: String,
    #[serde(rename(deserialize = "detail-type"))]
    detail_type: String,
    source: String,
    account: String,
    time: DateTime<Utc>,
    region: String,
    resources: Value,
    detail: Detail,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Detail {
    schema_version: String,
    account_id: String,
    region: String,
    partition: String,
    id: String,
    arn: String,
    #[serde(rename(deserialize = "type"))]
    tipe: String, // Type is a reserved Rust word, so we misspell it
    resource: Value,
    service: Service,
    severity: f32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    title: String,
    description: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Service {
    service_name: String,
    detector_id: String,
    action: Value,
    resource_role: String,
    additional_info: Value,
    event_first_seen: DateTime<Utc>,
    event_last_seen: DateTime<Utc>,
    archived: bool,
    count: usize,
}

impl Message {
    /// Sets up the Slack payload, constructed loosely around the Slack BlockKit format.
    /// Note you can't repeat things, like .text().text()
    ///
    /// This is built using the excellent slack-hook3 crate, this is a branch of a fork of a fork because
    /// the original maintainer doesn't look after slack-hook any more.
    /// https://github.com/0xc0deface/rust-slack/tree/v3

    fn build_payload(&self) -> Payload {
        let levels = Levels::default();
        let level = levels.from_severity(self.detail.severity);

        let fallback = format!(
            "GuardDuty:{} in {} {}",
            self.detail.tipe, self.detail.account_id, self.detail.region
        );

        let fields = vec![
            Field {
                title: String::from("Severity"),
                value: self.detail.severity.to_string().into(),
                short: Some(true),
            },
            Field {
                title: String::from("First seen"),
                value: self
                    .detail
                    .service
                    .event_first_seen
                    .format("%a %b %e %T")
                    .to_string()
                    .into(),
                short: Some(true),
            },
            Field {
                title: String::from("Count"),
                value: self.detail.service.count.to_string().into(),
                short: Some(true),
            },
            Field {
                title: String::from("Last seen"),
                value: self
                    .detail
                    .service
                    .event_last_seen
                    .format("%a %b %e %T")
                    .to_string()
                    .into(),
                short: Some(true),
            },
        ];
        let a = AttachmentBuilder::new(fallback)
            .color(&*level.colour)
            .pretext(format!(
                "*Finding in {} from account {}* {}",
                &self.detail.region, &self.detail.account_id, &*level.mention
            ))
            .title(&*self.detail.tipe)
            .title_link(&self.finding_link())
            .text(&*self.detail.description)
            .fields(fields)
            .footer("GuardyBot")
            .footer_icon("https://rustacean.net/assets/rustacean-flat-happy.png")
            .ts(&self.detail.updated_at.naive_local())
            .build()
            .expect("ERR: Failed to build Slack attachment");

        PayloadBuilder::new()
            .attachments(vec![a])
            .link_names(true)
            .build()
            .expect("ERR: Failed to build Slack payload")
    }

    /// Performs the required transformation to turn an AWS finding name string into the correct
    /// URL to their GuardDuty docs. These aren't all deterministic (IAMUser links to iam.html)
    /// Deliberately setup in a way to fail if a new finding category comes out, so we don't start sending
    /// bad links.
    fn finding_link(&self) -> String {
        let finding = &self.detail.tipe;
        let base_url = "https://docs.aws.amazon.com/guardduty/latest/ug/guardduty_finding-types-";

        let re = Regex::new(r"(?::)([\w]*?)(?:/)").unwrap(); // Capture "bar" from "foo:bar/baz"
        let lower_finding = &finding.to_lowercase(); // Downcase the string
        let caps = re.captures(lower_finding); // ec2 or None

        let finding_group = match caps {
            Some(caps) => caps.get(1),
            None => {
                log::error!("ERR: Couldn't match a finding group in: {}", &finding);
                return String::from("");
            }
        };

        let group_str = match finding_group {
            None => return String::from(""),
            Some(s) if s.as_str() == "iamuser" => String::from("iam"),
            Some(s) if s.as_str() == "ec2" => String::from("ec2"),
            Some(s) if s.as_str() == "s3" => String::from("s3"),
            Some(s) if s.as_str() == "kubernetes" => String::from("kubernetes"),
            Some(s) => {
                log::error!("ERR: Got unexpected finding group: {:#?}", s.to_owned());
                return String::from("");
            }
        };

        let anchor = re.replace(lower_finding, format!("-{}-", &group_str));

        return format!("{}{}.html#{}", base_url, group_str, anchor);
    }
}
struct SeverityLevel<'a> {
    colour: &'a str,
    mention: &'a str,
}

struct Levels<'a> {
    critical: SeverityLevel<'a>,
    high: SeverityLevel<'a>,
    medium: SeverityLevel<'a>,
    low: SeverityLevel<'a>,
    unknown: SeverityLevel<'a>,
}

impl<'a> Default for Levels<'a> {
    fn default() -> Levels<'a> {
        Levels {
            critical: SeverityLevel {
                colour: Colour::RED,
                mention: "@channel",
            },
            high: SeverityLevel {
                colour: Colour::ORANGE,
                mention: "@channel",
            },
            medium: SeverityLevel {
                colour: Colour::YELLOW,
                mention: "@here",
            },
            low: SeverityLevel {
                colour: Colour::BLUE,
                mention: "",
            },
            unknown: SeverityLevel {
                colour: Colour::SILVER,
                mention: "",
            },
        }
    }
}

impl<'a> Levels<'a> {
    fn from_severity(self, severity: f32) -> SeverityLevel<'a> {
        return match severity {
            x if x >= 9.0 && x < 10.0 => self.critical,
            x if x >= 7.0 && x < 9.0 => self.high,
            x if x >= 4.0 && x < 7.0 => self.medium,
            x if x >= 1.0 && x < 4.0 => self.low,
            _ => self.unknown,
        };
    }
}

#[non_exhaustive]
enum Colour {}
#[allow(dead_code)]
impl Colour {
    pub const RED: &'static str = "#DF4661";
    pub const ORANGE: &'static str = "#DB6B30";
    pub const YELLOW: &'static str = "#FED141";
    pub const GREEN: &'static str = "#008C95";
    pub const BLUE: &'static str = "#00A3E0";
    pub const SILVER: &'static str = "#BABABA";
    pub const PINK: &'static str = "#AF1685";
    pub const PURPLE: &'static str = "#2E1A47";
}
