/// A text layer's content: literals interleaved with `{name}` / `{name:arg}`
/// variables. `{{` and `}}` produce literal braces.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Literal(String),
    Var { name: String, arg: Option<String> },
}

/// How often a template's text can change, so the renderer only wakes when
/// it has to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cadence {
    Second,
    TwoSeconds,
    HalfMinute,
    Minute,
    Never,
}

const VARIABLES: &[&str] = &[
    "time", "date", "cpu", "mem", "mem_used", "load", "uptime", "battery", "host", "user", "var",
];

impl Template {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.chars().count() > 256 {
            return Err("text exceeds 256 characters".into());
        }
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '{' => {
                    let mut body = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => body.push(c),
                            None => return Err("unterminated {variable}".into()),
                        }
                    }
                    let (name, arg) = match body.split_once(':') {
                        Some((name, arg)) => (name.trim(), Some(arg.to_owned())),
                        None => (body.trim(), None),
                    };
                    if !VARIABLES.contains(&name) {
                        return Err(format!("unknown variable {{{name}}}"));
                    }
                    if name == "var"
                        && !arg.as_deref().is_some_and(|a| {
                            !a.is_empty()
                                && a.len() <= 32
                                && a.chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                        })
                    {
                        return Err("{var:name} needs a name of [A-Za-z0-9_-]".into());
                    }
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(Part::Var {
                        name: name.into(),
                        arg,
                    });
                }
                '}' => return Err("unmatched }".into()),
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        Ok(Self { parts })
    }

    pub fn cadence(&self) -> Cadence {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Literal(_) => None,
                Part::Var { name, arg } => Some(match name.as_str() {
                    "time" | "date" => {
                        let format = arg.as_deref().unwrap_or("");
                        if ["%S", "%T", "%s", "%r", "%X", "%c"]
                            .iter()
                            .any(|s| format.contains(s))
                        {
                            Cadence::Second
                        } else {
                            Cadence::Minute
                        }
                    }
                    "cpu" | "mem" | "mem_used" | "load" => Cadence::TwoSeconds,
                    "battery" => Cadence::HalfMinute,
                    "uptime" => Cadence::Minute,
                    _ => Cadence::Never,
                }),
            })
            .min()
            .unwrap_or(Cadence::Never)
    }

    /// Expands the template; `value` receives a variable's name and argument.
    pub fn render(&self, mut value: impl FnMut(&str, Option<&str>) -> String) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => out.push_str(text),
                Part::Var { name, arg } => out.push_str(&value(name, arg.as_deref())),
            }
        }
        out
    }
}
