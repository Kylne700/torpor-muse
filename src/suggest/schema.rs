//! The flat schema Muse Spark answers with, the typed answer, and the conversion to the engine's check JSON.
//! The converter does not judge the check: the engine's own lint does, and its words go back to the model once.
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// A start command the model may pick. The model sees only `key`; the check runs `cmd`.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub key: String,
    pub cmd: String,
}

/// The only environment the writer may set (a fixed list, never free text).
pub const NODE_ENVS: [&str; 3] = ["", "development", "test"];
const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];
const TYPES: [&str; 6] = ["string", "number", "boolean", "array", "object", "null"];
const START_TIMEOUT_S: u32 = 30;

/// The `text.format` object of the request. Flat: depth 6, no recursion, no keyword beyond type/enum/required.
pub fn response_format(starts: &[Start]) -> Value {
    let mut scripts: Vec<&str> = starts.iter().map(|s| s.key.as_str()).collect();
    scripts.push("");
    let t = || json!({"type": "string"});
    json!({
        "type": "json_schema",
        "name": "torpor_muse_check",
        "strict": true,
        "schema": {
            "type": "object",
            "additionalProperties": false,
            "required": ["verdict", "reason", "name", "why", "start_script", "ready_path", "node_env", "steps"],
            "properties": {
                "verdict": {"type": "string", "enum": ["check", "not_a_claim", "cannot_check"]},
                "reason": t(),
                "name": t(),
                "why": t(),
                "start_script": {"type": "string", "enum": scripts},
                "ready_path": t(),
                "node_env": {"type": "string", "enum": NODE_ENVS},
                "steps": {"type": "array", "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["method", "path", "body_json", "expect_status", "json_has", "json_types", "body_contains", "why"],
                    "properties": {
                        "method": {"type": "string", "enum": METHODS},
                        "path": t(),
                        "body_json": t(),
                        "expect_status": {"type": "integer"},
                        "json_has": {"type": "array", "items": t()},
                        "json_types": {"type": "array", "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["path", "type"],
                            "properties": {"path": t(), "type": {"type": "string", "enum": TYPES}}
                        }},
                        "body_contains": t(),
                        "why": t()
                    }
                }}
            }
        }
    })
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Check,
    NotAClaim,
    CannotCheck,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    String,
    Number,
    Boolean,
    Array,
    Object,
    Null,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::String => "string",
            Kind::Number => "number",
            Kind::Boolean => "boolean",
            Kind::Array => "array",
            Kind::Object => "object",
            Kind::Null => "null",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonType {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: Kind,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub method: Method,
    pub path: String,
    pub body_json: String,
    pub expect_status: i64,
    pub json_has: Vec<String>,
    pub json_types: Vec<JsonType>,
    pub body_contains: String,
    pub why: String,
}

/// The model's whole answer. A key that is missing, extra or of the wrong type is an error: never a half check.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub verdict: Verdict,
    pub reason: String,
    pub name: String,
    pub why: String,
    pub start_script: String,
    pub ready_path: String,
    pub node_env: String,
    pub steps: Vec<Step>,
}

pub fn parse(text: &str) -> Result<Answer, String> {
    serde_json::from_str(text).map_err(|e| format!("the answer does not fit the schema: {e}"))
}

/// Takes out the steps whose path names a record already in the user's data, one the check did not create and that may
/// be gone tomorrow (dogfood1 M5: a note id from the data). Such a path part (before `?`, without `{rand}`) has a digit
/// and is all digits or no whole word of the code the model saw. Returns them as "step N (METHOD path)".
/// shortcut: names without a digit (`/users/alice`) and ids in a body or a query are not caught; upgrade when a saved
/// check is seen to depend on one.
pub fn drop_data_records(a: &mut Answer, excerpt: &str) -> Vec<String> {
    let words: std::collections::HashSet<&str> = excerpt
        .split(|c: char| !c.is_ascii_alphanumeric())
        .collect();
    let foreign = |t: &str| {
        t.bytes().any(|b| b.is_ascii_digit())
            && (t.bytes().all(|b| b.is_ascii_digit()) || !words.contains(t))
    };
    let (mut gone, mut n) = (Vec::new(), 0);
    a.steps.retain(|st| {
        n += 1;
        let path = st.path.split('?').next().unwrap_or("");
        let hit = path
            .split('/')
            .filter(|part| !part.contains("{rand}"))
            .flat_map(|part| part.split(|c: char| !c.is_ascii_alphanumeric()))
            .any(foreign);
        if hit {
            gone.push(format!("step {n} ({} {path})", st.method.as_str()));
        }
        !hit
    });
    gone
}

/// Builds the engine's check from a `check` answer. Empty strings mean "not set". Only what the schema cannot say is
/// checked here (the allowlisted start and env, a body that is JSON text); the engine's lint judges the rest.
pub fn to_check(a: &Answer, starts: &[Start]) -> Result<Value, String> {
    if a.verdict != Verdict::Check {
        return Err("the verdict is not `check`".into());
    }
    if a.steps.is_empty() {
        return Err("the verdict is `check` but there are no steps".into());
    }
    let start = starts
        .iter()
        .find(|s| s.key == a.start_script)
        .ok_or_else(|| {
            let keys: Vec<&str> = starts.iter().map(|s| s.key.as_str()).collect();
            format!("start_script must be one of {keys:?}")
        })?;
    if !NODE_ENVS.contains(&a.node_env.as_str()) {
        return Err(format!("node_env must be one of {NODE_ENVS:?}"));
    }
    let mut steps = Vec::new();
    for (i, st) in a.steps.iter().enumerate() {
        let n = i + 1;
        let mut http = Map::new();
        http.insert("method".into(), st.method.as_str().into());
        http.insert("path".into(), st.path.clone().into());
        if !st.body_json.is_empty() {
            let body: Value = serde_json::from_str(&st.body_json)
                .map_err(|e| format!("step {n}: body_json is not JSON text ({e})"))?;
            http.insert("json".into(), body);
        }
        let mut expect = Map::new();
        if st.expect_status != 0 {
            if !(100..=599).contains(&st.expect_status) {
                return Err(format!("step {n}: expect_status is not an HTTP status"));
            }
            expect.insert("status".into(), st.expect_status.into());
        }
        if !st.json_has.is_empty() {
            expect.insert("json_has".into(), json!(st.json_has));
        }
        if !st.json_types.is_empty() {
            let m: Map<String, Value> = st
                .json_types
                .iter()
                .map(|j| (j.path.clone(), j.kind.as_str().into()))
                .collect();
            expect.insert("json_type".into(), m.into());
        }
        if !st.body_contains.is_empty() {
            expect.insert("body_contains".into(), st.body_contains.clone().into());
        }
        steps.push(json!({"http": http, "expect": expect, "why": st.why}));
    }
    let mut check = json!({
        "v": 1,
        "name": a.name,
        "why": a.why,
        "start": {"cmd": start.cmd, "ready": a.ready_path, "timeout_s": START_TIMEOUT_S},
        "steps": steps,
    });
    if !a.node_env.is_empty() {
        check["env"] = json!({"NODE_ENV": a.node_env});
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts() -> Vec<Start> {
        vec![Start {
            key: "start".into(),
            cmd: "npm start --silent".into(),
        }]
    }

    const GOOD: &str = r#"{"verdict":"check","reason":"","name":"login works","why":"you said login works",
      "start_script":"start","ready_path":"/login","node_env":"",
      "steps":[{"method":"POST","path":"/api/login","body_json":"{\"user\":\"alice\",\"password\":\"x\"}",
        "expect_status":200,"json_has":["user"],"json_types":[{"path":"user","type":"string"}],
        "body_contains":"","why":"login is accepted"}]}"#;

    fn depth(v: &Value) -> usize {
        match v {
            Value::Object(m) => 1 + m.values().map(depth).max().unwrap_or(0),
            Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
            _ => 0,
        }
    }

    #[test]
    fn the_schema_is_flat_strict_and_closed() {
        let f = response_format(&starts());
        let text = f.to_string();
        assert!(depth(&f["schema"]) <= 10, "depth {}", depth(&f["schema"]));
        assert!(!text.contains("$ref") && !text.contains("anyOf") && !text.contains("maxLength"));
        // strict mode: every object lists all its properties as required and forbids extra ones
        fn closed(v: &Value) {
            if let Value::Object(m) = v {
                if m.get("type") == Some(&json!("object")) {
                    let props: Vec<&String> = m["properties"].as_object().unwrap().keys().collect();
                    let req: Vec<&str> = m["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_str().unwrap())
                        .collect();
                    assert_eq!(props.len(), req.len());
                    assert!(props.iter().all(|p| req.contains(&p.as_str())));
                    assert_eq!(m["additionalProperties"], json!(false));
                }
                m.values().for_each(closed);
            }
            if let Value::Array(a) = v {
                a.iter().for_each(closed);
            }
        }
        closed(&f["schema"]);
        assert_eq!(
            f["schema"]["properties"]["start_script"]["enum"],
            json!(["start", ""])
        );
    }

    #[test]
    fn a_good_answer_becomes_an_engine_check() {
        let a = parse(GOOD).unwrap();
        let c = to_check(&a, &starts()).unwrap();
        assert_eq!(c["v"], 1);
        assert_eq!(c["start"]["cmd"], "npm start --silent");
        assert_eq!(c["start"]["ready"], "/login");
        assert_eq!(c["steps"][0]["http"]["method"], "POST");
        assert_eq!(c["steps"][0]["http"]["json"]["user"], "alice");
        assert_eq!(c["steps"][0]["expect"]["status"], 200);
        assert_eq!(c["steps"][0]["expect"]["json_type"]["user"], "string");
        assert!(c["steps"][0]["expect"].get("body_contains").is_none());
        assert!(c.get("env").is_none());
    }

    #[test]
    fn broken_outputs_are_errors_never_a_half_check() {
        let extra = GOOD.replace("\"reason\":\"\",", "\"reason\":\"\",\"sneaky\":1,");
        assert!(parse(&extra).unwrap_err().contains("sneaky"));
        let wrong_type = GOOD.replace("\"expect_status\":200", "\"expect_status\":\"200\"");
        assert!(parse(&wrong_type).is_err());
        let missing = GOOD.replace("\"node_env\":\"\",", "");
        assert!(parse(&missing).is_err());
        let bad_verdict = GOOD.replace("\"check\"", "\"maybe\"");
        assert!(parse(&bad_verdict).is_err());
        assert!(parse(&format!("{GOOD} trailing")).is_err());
        assert!(parse(&format!("```json\n{GOOD}\n```")).is_err());
        assert!(parse(&GOOD[..GOOD.len() / 2]).is_err());
        assert!(parse("").is_err());
        assert!(parse("[]").is_err());
    }

    #[test]
    fn what_the_schema_cannot_say_is_checked_here() {
        let mut a = parse(GOOD).unwrap();
        a.start_script = "rm -rf".into();
        assert!(
            to_check(&a, &starts())
                .unwrap_err()
                .contains("start_script")
        );
        let mut a = parse(GOOD).unwrap();
        a.node_env = "production".into();
        assert!(to_check(&a, &starts()).unwrap_err().contains("node_env"));
        let mut a = parse(GOOD).unwrap();
        a.steps[0].body_json = "{not json".into();
        assert!(to_check(&a, &starts()).unwrap_err().contains("body_json"));
        let mut a = parse(GOOD).unwrap();
        a.steps[0].expect_status = 999;
        assert!(to_check(&a, &starts()).is_err());
        let mut a = parse(GOOD).unwrap();
        a.steps.clear();
        assert!(to_check(&a, &starts()).is_err());
        let mut a = parse(GOOD).unwrap();
        a.node_env = "test".into();
        assert_eq!(to_check(&a, &starts()).unwrap()["env"]["NODE_ENV"], "test");
    }
}
