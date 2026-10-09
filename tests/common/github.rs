//! A local server standing in for the GitHub releases API of `acme/plugins`: lists releases,
//! creates them, takes uploads and deletions, serves assets. Shared by library and CLI tests.

#![allow(dead_code)]

use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::Request;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;
use wiremock::matchers::path_regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Normal,
    UploadRefused(u16),
    MalformedListing,
    MalformedRelease,
    MalformedUpload,
}

fn answer(
    given: Answer,
    malformed_when: Answer,
    status: u16,
    body: serde_json::Value,
) -> ResponseTemplate {
    let template = ResponseTemplate::new(status);
    if given == malformed_when {
        template.set_body_string("not json")
    } else {
        template.set_body_json(body)
    }
}

pub type Release<'a> = (&'a str, &'a [(&'a str, &'a [u8])]);

pub async fn fake_github(existing: &[Release<'_>], given: Answer) -> MockServer {
    let upload_status = if let Answer::UploadRefused(status) = given {
        status
    } else {
        201
    };
    let server = MockServer::start().await;
    let uri = server.uri();
    let listing: Vec<_> = existing
        .iter()
        .map(|(tag, assets)| {
            let assets: Vec<_> = assets.iter().map(|(name, _)| json!({ "name": name, "url": format!("{uri}/assets/{tag}/{name}") })).collect();
            json!({ "tag_name": tag, "upload_url": format!("{uri}/upload/{tag}{{?name,label}}"), "assets": assets })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/repos/acme/plugins/releases"))
        .respond_with(answer(given, Answer::MalformedListing, 200, json!(listing)))
        .mount(&server)
        .await;
    let base = uri.clone();
    let create = move |request: &Request| {
        let tag = request.body_json::<serde_json::Value>().unwrap()["tag_name"]
            .as_str()
            .unwrap()
            .to_owned();
        answer(
            given,
            Answer::MalformedRelease,
            201,
            json!({ "tag_name": tag, "upload_url": format!("{base}/upload/{tag}{{?name,label}}"), "assets": [] }),
        )
    };
    Mock::given(method("POST"))
        .and(path("/repos/acme/plugins/releases"))
        .respond_with(create)
        .mount(&server)
        .await;
    let base = uri.clone();
    let upload = move |request: &Request| {
        let tag = request.url.path().trim_start_matches("/upload/").to_owned();
        let name = request
            .url
            .query_pairs()
            .find(|(key, _)| key == "name")
            .unwrap()
            .1
            .into_owned();
        answer(
            given,
            Answer::MalformedUpload,
            upload_status,
            json!({ "name": name, "url": format!("{base}/assets/{tag}/{name}") }),
        )
    };
    Mock::given(method("POST"))
        .and(path_regex("^/upload/"))
        .respond_with(upload)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex("^/assets/"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    for (tag, assets) in existing {
        for (name, body) in *assets {
            let served = ResponseTemplate::new(200).set_body_bytes(body.to_vec());
            Mock::given(method("GET"))
                .and(path(format!("/assets/{tag}/{name}")))
                .respond_with(served)
                .mount(&server)
                .await;
        }
    }
    server
}

pub async fn writes(server: &MockServer) -> Vec<String> {
    let requests = server.received_requests().await.unwrap();
    let written = requests.iter().filter(|r| r.method.as_str() != "GET");
    written
        .map(|r| {
            format!(
                "{} {}{}",
                r.method,
                r.url.path(),
                r.url.query().map(|q| format!("?{q}")).unwrap_or_default()
            )
        })
        .collect()
}

pub async fn uploads(server: &MockServer) -> std::collections::HashMap<String, Vec<u8>> {
    let requests = server.received_requests().await.unwrap();
    let uploads = requests
        .into_iter()
        .filter(|r| r.url.path().starts_with("/upload/"));
    uploads
        .map(|r| {
            (
                r.url
                    .query_pairs()
                    .find(|(key, _)| key == "name")
                    .unwrap()
                    .1
                    .into_owned(),
                r.body,
            )
        })
        .collect()
}
