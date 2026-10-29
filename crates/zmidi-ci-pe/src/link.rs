//! Resource Link Properties (M2-103-UM v1.2 section 13; Tables 64, 68 and 72).
//!
//! - [`Link::parse`] reads one Link object of a "links" array with no copy: strings stay as
//!   escaped JSON text ([`HeaderStr`]) and "propData" stays raw JSON text (DEC-007). The caller
//!   decodes "title" or "confirmMsg" into its own buffer with `HeaderStr::decode_into`, which
//!   cuts a long value at a character boundary and marks it (REQ-PE-LINK-002).
//! - The role is plain (no "role"), "simpleAction", "selectorAction" or another role; the
//!   comparison is case-sensitive (REQ-PE-LINK-003).
//! - [`Link::write`] writes a Link with the writer rules of REQ-PE-LINK-001 to -007.

use crate::header::HeaderStr;
use crate::json::{JsonError, JsonWriter};

const MAX_RESOURCE: usize = 36;
const MAX_ROLE: usize = 32;

/// Error of the Link reader and writer; it names the Property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// The Link is not a valid JSON object.
    #[error("Link JSON: {0}")]
    Json(JsonError),
    /// A required Property is missing.
    #[error("missing \"{0}\"")]
    Missing(&'static str),
    /// A Property has a wrong type, length or character.
    #[error("invalid \"{0}\"")]
    Invalid(&'static str),
}

impl From<JsonError> for LinkError {
    fn from(e: JsonError) -> Self {
        LinkError::Json(e)
    }
}

/// The role of a Link (REQ-PE-LINK-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRole<'a> {
    /// No "role": a plain link.
    Plain,
    /// "simpleAction".
    SimpleAction,
    /// "selectorAction".
    SelectorAction,
    /// Another role, with its string.
    Other(HeaderStr<'a>),
}

/// "confirmType" of a "simpleAction" Link (REQ-PE-LINK-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmType<'a> {
    /// "okcancel".
    OkCancel,
    /// "yesno".
    YesNo,
    /// Another value; the writer refuses it.
    Unknown(HeaderStr<'a>),
}

/// "confirmIcon" of a "simpleAction" Link (REQ-PE-LINK-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmIcon<'a> {
    /// "exclamation".
    Exclamation,
    /// "warning".
    Warning,
    /// "information".
    Information,
    /// "question".
    Question,
    /// Another value; the writer refuses it.
    Unknown(HeaderStr<'a>),
}

/// One Link object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link<'a> {
    /// "resource", 1-36 characters, required.
    pub resource: HeaderStr<'a>,
    /// "resId", up to 36 characters.
    pub res_id: Option<HeaderStr<'a>>,
    /// "title", no fixed maximum.
    pub title: Option<HeaderStr<'a>>,
    /// "role".
    pub role: LinkRole<'a>,
    /// "propData" of a "simpleAction" Link, raw JSON text.
    pub prop_data: Option<&'a [u8]>,
    /// "confirmMsg", no fixed maximum.
    pub confirm_msg: Option<HeaderStr<'a>>,
    /// "confirmType".
    pub confirm_type: Option<ConfirmType<'a>>,
    /// "confirmIcon".
    pub confirm_icon: Option<ConfirmIcon<'a>>,
    /// "actionResource" of a "selectorAction" Link, 1-36 characters.
    pub action_resource: Option<HeaderStr<'a>>,
    /// "actionResId" of a "selectorAction" Link, up to 36 characters.
    pub action_res_id: Option<HeaderStr<'a>>,
    /// "selectedProperty" of a "selectorAction" Link, at least 1 character.
    pub selected_property: Option<HeaderStr<'a>>,
}

const NAMES: [&str; 11] = [
    "resource",
    "resId",
    "title",
    "role",
    "propData",
    "confirmMsg",
    "confirmType",
    "confirmIcon",
    "actionResource",
    "actionResId",
    "selectedProperty",
];

fn string<'a>(raw: &'a [u8], name: &'static str) -> Result<HeaderStr<'a>, LinkError> {
    match raw {
        [b'"', inner @ .., b'"'] => Ok(HeaderStr::Raw(inner)),
        _ => Err(LinkError::Invalid(name)),
    }
}

fn max_chars<'a>(
    v: HeaderStr<'a>,
    min: usize,
    max: usize,
    name: &'static str,
) -> Result<HeaderStr<'a>, LinkError> {
    if (min..=max).contains(&v.char_count()) {
        Ok(v)
    } else {
        Err(LinkError::Invalid(name))
    }
}

impl<'a> Link<'a> {
    /// Reads one Link object (REQ-PE-LINK-001 to -007). Unknown Properties are skipped.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, LinkError> {
        let mut resource = None;
        let mut l = Link {
            resource: HeaderStr::Text(""),
            res_id: None,
            title: None,
            role: LinkRole::Plain,
            prop_data: None,
            confirm_msg: None,
            confirm_type: None,
            confirm_icon: None,
            action_resource: None,
            action_res_id: None,
            selected_property: None,
        };
        crate::json::for_each_member(bytes, |name, raw| -> Result<(), LinkError> {
            let Some(name) = name.and_then(|n| NAMES.into_iter().find(|k| *k == n)) else {
                return Ok(());
            };
            match name {
                "resource" => {
                    resource = Some(max_chars(string(raw, name)?, 1, MAX_RESOURCE, name)?)
                }
                "resId" => l.res_id = Some(max_chars(string(raw, name)?, 0, MAX_RESOURCE, name)?),
                "title" => l.title = Some(string(raw, name)?),
                "role" => {
                    let v = max_chars(string(raw, name)?, 0, MAX_ROLE, name)?;
                    l.role = if v.eq_str("simpleAction") {
                        LinkRole::SimpleAction
                    } else if v.eq_str("selectorAction") {
                        LinkRole::SelectorAction
                    } else {
                        LinkRole::Other(v)
                    };
                }
                // The reader checked its syntax and its depth (64).
                "propData" => l.prop_data = Some(raw),
                "confirmMsg" => l.confirm_msg = Some(string(raw, name)?),
                "confirmType" => {
                    let v = string(raw, name)?;
                    l.confirm_type = Some(match () {
                        _ if v.eq_str("okcancel") => ConfirmType::OkCancel,
                        _ if v.eq_str("yesno") => ConfirmType::YesNo,
                        _ => ConfirmType::Unknown(v),
                    });
                }
                "confirmIcon" => {
                    let v = string(raw, name)?;
                    l.confirm_icon = Some(match () {
                        _ if v.eq_str("exclamation") => ConfirmIcon::Exclamation,
                        _ if v.eq_str("warning") => ConfirmIcon::Warning,
                        _ if v.eq_str("information") => ConfirmIcon::Information,
                        _ if v.eq_str("question") => ConfirmIcon::Question,
                        _ => ConfirmIcon::Unknown(v),
                    });
                }
                "actionResource" => {
                    l.action_resource = Some(max_chars(string(raw, name)?, 1, MAX_RESOURCE, name)?)
                }
                "actionResId" => {
                    l.action_res_id = Some(max_chars(string(raw, name)?, 0, MAX_RESOURCE, name)?)
                }
                "selectedProperty" => {
                    l.selected_property = Some(max_chars(string(raw, name)?, 1, usize::MAX, name)?)
                }
                _ => {}
            }
            Ok(())
        })?;
        l.resource = resource.ok_or(LinkError::Missing("resource"))?;
        match l.role {
            LinkRole::SimpleAction if l.prop_data.is_none() => Err(LinkError::Missing("propData")),
            LinkRole::SelectorAction if l.action_resource.is_none() => {
                Err(LinkError::Missing("actionResource"))
            }
            LinkRole::SelectorAction if l.selected_property.is_none() => {
                Err(LinkError::Missing("selectedProperty"))
            }
            _ => Ok(l),
        }
    }

    /// Writes the Link as one JSON object into `out` (REQ-PE-LINK-001 to -007). Caller text is
    /// checked; received raw strings are written back unchanged.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, LinkError> {
        let text = |v: &HeaderStr<'_>| match v {
            HeaderStr::Text(t) => Some(t.chars().count()),
            HeaderStr::Raw(_) => None,
        };
        let id_ok = |v: &Option<HeaderStr<'_>>| match v {
            Some(HeaderStr::Text(t)) => {
                t.len() <= MAX_RESOURCE
                    && t.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            }
            _ => true,
        };
        if text(&self.resource).is_some_and(|n| !(1..=MAX_RESOURCE).contains(&n)) {
            return Err(LinkError::Invalid("resource"));
        }
        if self
            .res_id
            .as_ref()
            .and_then(text)
            .is_some_and(|n| n > MAX_RESOURCE)
        {
            return Err(LinkError::Invalid("resId"));
        }
        let action = matches!(self.role, LinkRole::SimpleAction | LinkRole::SelectorAction);
        if action && !id_ok(&self.res_id) {
            return Err(LinkError::Invalid("resId"));
        }
        if !id_ok(&self.action_res_id) {
            return Err(LinkError::Invalid("actionResId"));
        }
        if self
            .action_resource
            .as_ref()
            .and_then(text)
            .is_some_and(|n| !(1..=MAX_RESOURCE).contains(&n))
        {
            return Err(LinkError::Invalid("actionResource"));
        }
        // REQ-PE-LINK-007 (Table 72): "selectedProperty" has at least 1 character.
        if self.selected_property.as_ref().and_then(text) == Some(0) {
            return Err(LinkError::Invalid("selectedProperty"));
        }
        if matches!(self.role, LinkRole::Other(r) if text(&r).is_some_and(|n| n > MAX_ROLE)) {
            return Err(LinkError::Invalid("role"));
        }
        if matches!(self.confirm_type, Some(ConfirmType::Unknown(_))) {
            return Err(LinkError::Invalid("confirmType"));
        }
        if matches!(self.confirm_icon, Some(ConfirmIcon::Unknown(_))) {
            return Err(LinkError::Invalid("confirmIcon"));
        }
        match self.role {
            LinkRole::SimpleAction if self.prop_data.is_none() => {
                return Err(LinkError::Missing("propData"))
            }
            LinkRole::SelectorAction if self.action_resource.is_none() => {
                return Err(LinkError::Missing("actionResource"))
            }
            LinkRole::SelectorAction if self.selected_property.is_none() => {
                return Err(LinkError::Missing("selectedProperty"))
            }
            _ => {}
        }
        let mut w = JsonWriter::new(out);
        w.begin_object()?;
        let put = |w: &mut JsonWriter<'_>,
                   name: &str,
                   v: &Option<HeaderStr<'_>>|
         -> Result<(), JsonError> {
            if let Some(v) = v {
                w.name(name, 20)?;
                put_str(w, v)?;
            }
            Ok(())
        };
        put(&mut w, "resource", &Some(self.resource))?;
        put(&mut w, "resId", &self.res_id)?;
        put(&mut w, "title", &self.title)?;
        match self.role {
            LinkRole::Plain => {}
            LinkRole::SimpleAction => put(&mut w, "role", &Some(HeaderStr::Text("simpleAction")))?,
            LinkRole::SelectorAction => {
                put(&mut w, "role", &Some(HeaderStr::Text("selectorAction")))?
            }
            LinkRole::Other(r) => put(&mut w, "role", &Some(r))?,
        }
        if let Some(p) = self.prop_data {
            w.name("propData", 20)?;
            w.raw(p)?;
        }
        put(&mut w, "confirmMsg", &self.confirm_msg)?;
        let ct = self.confirm_type.map(|c| match c {
            ConfirmType::OkCancel => "okcancel",
            _ => "yesno",
        });
        put(&mut w, "confirmType", &ct.map(HeaderStr::Text))?;
        let ci = self.confirm_icon.map(|c| match c {
            ConfirmIcon::Exclamation => "exclamation",
            ConfirmIcon::Warning => "warning",
            ConfirmIcon::Information => "information",
            _ => "question",
        });
        put(&mut w, "confirmIcon", &ci.map(HeaderStr::Text))?;
        put(&mut w, "actionResource", &self.action_resource)?;
        put(&mut w, "actionResId", &self.action_res_id)?;
        put(&mut w, "selectedProperty", &self.selected_property)?;
        w.end_object()?;
        Ok(w.finish()?)
    }
}

fn put_str(w: &mut JsonWriter<'_>, v: &HeaderStr<'_>) -> Result<(), JsonError> {
    match v {
        HeaderStr::Text(t) => w.string(t),
        HeaderStr::Raw(_) => w.string_chars(v.chars()),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::string::String;

    fn s(v: Option<HeaderStr<'_>>) -> String {
        v.map(|v| v.chars().collect()).unwrap_or_default()
    }

    #[test]
    fn plain_and_action_links() {
        // M2-103 Tables 64, 68, 72; REQ-PE-LINK-001, -003, -005, -006, -007.
        let l =
            Link::parse(br#"{"resource":"ChannelList","title":"Channels","x":{"y":[1]}}"#).unwrap();
        assert_eq!(s(Some(l.resource)), "ChannelList");
        assert_eq!(l.role, LinkRole::Plain);
        assert_eq!(s(l.title), "Channels");
        let a = Link::parse(
            br#"{"resource":"X-Reset","role":"simpleAction","propData":{"reset":true,"n":[1,2]},"confirmMsg":"Sure?","confirmType":"yesno","confirmIcon":"warning"}"#,
        )
        .unwrap();
        assert_eq!(a.role, LinkRole::SimpleAction);
        assert_eq!(a.prop_data, Some(&br#"{"reset":true,"n":[1,2]}"#[..]));
        assert_eq!(
            (a.confirm_type, a.confirm_icon),
            (Some(ConfirmType::YesNo), Some(ConfirmIcon::Warning))
        );
        let sel = Link::parse(br#"{"resource":"ProgramList","role":"selectorAction","actionResource":"CurrentProgram","selectedProperty":"bankPC"}"#).unwrap();
        assert_eq!(sel.role, LinkRole::SelectorAction);
        assert_eq!(s(sel.selected_property), "bankPC");
        let other =
            Link::parse(br#"{"resource":"A","role":"selectAction","confirmIcon":"stop"}"#).unwrap();
        assert!(
            matches!(other.role, LinkRole::Other(r) if r.eq_str("selectAction")),
            "case-sensitive"
        );
        assert!(
            matches!(other.confirm_icon, Some(ConfirmIcon::Unknown(_))),
            "unknown value, no error"
        );
    }

    #[test]
    fn link_reader_errors() {
        // REQ-PE-LINK-001, -004, -005, -007.
        for bad in [
            &br#"{"title":"x"}"#[..],
            br#"{"resource":""}"#,
            br#"{"resource":"0123456789012345678901234567890123456"}"#,
            br#"{"resource":"A","role":"simpleAction"}"#,
            br#"{"resource":"A","role":"selectorAction","selectedProperty":"p"}"#,
            br#"{"resource":"A","role":"selectorAction","actionResource":"B"}"#,
            br#"{"resource":"A","role":"0123456789012345678901234567890123"}"#,
            br#"{"resource":"A","actionResId":"0123456789012345678901234567890123456"}"#,
            br#"{"resource":"A","propData":{"x":}}"#,
            br#"{"resource":1}"#,
        ] {
            assert!(
                Link::parse(bad).is_err(),
                "{}",
                core::str::from_utf8(bad).unwrap()
            );
        }
    }

    #[test]
    fn long_title_is_cut_by_the_caller_buffer() {
        // REQ-PE-LINK-002.
        let l = Link::parse(br#"{"resource":"A","title":"abcdefghij"}"#).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(l.title.unwrap().decode_into(&mut buf), ("abcd", true));
    }

    #[test]
    fn link_writer_rules() {
        // REQ-PE-LINK-001, -003, -004, -006, -007.
        let base = Link {
            resource: HeaderStr::Text("X-Reset"),
            res_id: None,
            title: Some(HeaderStr::Text("Reset")),
            role: LinkRole::SimpleAction,
            prop_data: Some(br#"{"reset":true}"#),
            confirm_msg: None,
            confirm_type: Some(ConfirmType::OkCancel),
            confirm_icon: None,
            action_resource: None,
            action_res_id: None,
            selected_property: None,
        };
        let mut buf = [0u8; 200];
        let n = base.write(&mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            br#"{"resource":"X-Reset","title":"Reset","role":"simpleAction","propData":{"reset":true},"confirmType":"okcancel"}"#
        );
        let back = Link::parse(&buf[..n]).unwrap();
        assert_eq!(back.role, LinkRole::SimpleAction);
        assert!(Link {
            res_id: Some(HeaderStr::Text("Upper")),
            ..base
        }
        .write(&mut buf)
        .is_err());
        assert!(Link {
            confirm_type: Some(ConfirmType::Unknown(HeaderStr::Text("x"))),
            ..base
        }
        .write(&mut buf)
        .is_err());
        assert!(Link {
            prop_data: None,
            ..base
        }
        .write(&mut buf)
        .is_err());
        assert!(Link {
            role: LinkRole::SelectorAction,
            ..base
        }
        .write(&mut buf)
        .is_err());
        let mut small = [0u8; 8];
        assert_eq!(
            base.write(&mut small),
            Err(LinkError::Json(JsonError::Overflow))
        );
    }

    #[test]
    fn link_reader_limits_and_values() {
        // REQ-PE-LINK-001, -004, -005, -006, -007: the exact error of each refused value; the
        // reader accepts any characters and gives unknown "confirmType" and "confirmIcon" values.
        let r37 = "r".repeat(37);
        let deep = std::format!("{}{}", "[".repeat(65), "]".repeat(65));
        let cases: [(String, LinkError); 8] = [
            (
                std::format!(r#"{{"resource":"A","resId":"{r37}"}}"#),
                LinkError::Invalid("resId"),
            ),
            (
                std::format!(
                    r#"{{"resource":"A","role":"simpleAction","propData":{{}},"resId":"{r37}"}}"#
                ),
                LinkError::Invalid("resId"),
            ),
            (
                std::format!(
                    r#"{{"resource":"A","role":"selectorAction","actionResource":"B","selectedProperty":"p","actionResId":"{r37}"}}"#
                ),
                LinkError::Invalid("actionResId"),
            ),
            (
                std::format!(
                    r#"{{"resource":"A","role":"selectorAction","actionResource":"{r37}","selectedProperty":"p"}}"#
                ),
                LinkError::Invalid("actionResource"),
            ),
            (
                r#"{"resource":"A","role":"selectorAction","actionResource":"B","selectedProperty":""}"#
                    .into(),
                LinkError::Invalid("selectedProperty"),
            ),
            (
                r#"{"resource":"A","role":"selectorAction","actionResource":"","selectedProperty":"p"}"#
                    .into(),
                LinkError::Invalid("actionResource"),
            ),
            (
                std::format!(r#"{{"resource":"A","role":"simpleAction","propData":{deep}}}"#),
                LinkError::Json(JsonError::Depth),
            ),
            (
                r#"{"resource":"A","role":"simpleAction","propData":[1,]}"#.into(),
                LinkError::Json(JsonError::Syntax),
            ),
        ];
        for (text, want) in &cases {
            assert_eq!(Link::parse(text.as_bytes()), Err(*want), "{text}");
        }
        let r36 = "\u{e9}".repeat(35) + "-";
        let ok = std::format!(
            r#"{{"resource":"A","role":"selectorAction","resId":"{r36}","actionResource":"B","actionResId":"Ab-é","selectedProperty":"p","confirmType":"okcancel","confirmIcon":"exclamation"}}"#
        );
        let l = Link::parse(ok.as_bytes()).unwrap();
        assert_eq!(
            (s(l.res_id), s(l.action_res_id)),
            (r36.clone(), String::from("Ab-\u{e9}"))
        );
        assert_eq!(
            (l.confirm_type, l.confirm_icon),
            (Some(ConfirmType::OkCancel), Some(ConfirmIcon::Exclamation))
        );
        let ok = std::format!(
            r#"{{"resource":"A","role":"simpleAction","propData":{}{}}}"#,
            "[".repeat(10),
            "]".repeat(10)
        );
        assert!(Link::parse(ok.as_bytes()).is_ok());
        for (text, icon) in [
            ("information", ConfirmIcon::Information),
            ("question", ConfirmIcon::Question),
            ("warning", ConfirmIcon::Warning),
        ] {
            let t = std::format!(r#"{{"resource":"A","confirmIcon":"{text}"}}"#);
            assert_eq!(Link::parse(t.as_bytes()).unwrap().confirm_icon, Some(icon));
        }
        let l = Link::parse(br#"{"resource":"A","confirmType":"OkCancel"}"#).unwrap();
        assert!(
            matches!(l.confirm_type, Some(ConfirmType::Unknown(v)) if v.eq_str("OkCancel")),
            "unknown value, no error"
        );
    }

    #[test]
    fn long_title_and_confirm_msg_are_cut_at_a_character_boundary() {
        // REQ-PE-LINK-002: "é" (2 bytes) crosses the end of the caller buffer: the cut is before
        // it, marked; the other Properties are kept.
        let l = Link::parse(
            r#"{"resource":"A","role":"simpleAction","propData":{},"title":"abc\u00e9","confirmMsg":"Sure\u00e9?","confirmType":"yesno"}"#
                .as_bytes(),
        )
        .unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(l.title.unwrap().decode_into(&mut buf), ("abc", true));
        let mut buf = [0u8; 5];
        assert_eq!(l.confirm_msg.unwrap().decode_into(&mut buf), ("Sure", true));
        let mut buf = [0u8; 7];
        assert_eq!(
            l.confirm_msg.unwrap().decode_into(&mut buf),
            ("Sure\u{e9}?", false)
        );
        assert_eq!(s(Some(l.resource)), "A");
        assert_eq!(l.confirm_type, Some(ConfirmType::YesNo));
    }

    #[test]
    fn link_writer_limits_and_selector() {
        // REQ-PE-LINK-001, -003, -004, -006, -007: the exact error of each refused value, and the
        // bytes of a selectorAction Link.
        let sel = Link {
            resource: HeaderStr::Text("ProgramList"),
            res_id: None,
            title: None,
            role: LinkRole::SelectorAction,
            prop_data: None,
            confirm_msg: None,
            confirm_type: None,
            confirm_icon: None,
            action_resource: Some(HeaderStr::Text("CurrentProgram")),
            action_res_id: Some(HeaderStr::Text("p_1")),
            selected_property: Some(HeaderStr::Text("bankPC")),
        };
        let mut buf = [0u8; 200];
        let n = sel.write(&mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            br#"{"resource":"ProgramList","role":"selectorAction","actionResource":"CurrentProgram","actionResId":"p_1","selectedProperty":"bankPC"}"#
        );
        let plain = Link {
            role: LinkRole::Plain,
            action_resource: None,
            action_res_id: None,
            selected_property: None,
            ..sel
        };
        let a36 = "a".repeat(36);
        let a37 = "a".repeat(37);
        let r33 = "r".repeat(33);
        let r32 = "r".repeat(32);
        let refused: [(Link<'_>, LinkError); 11] = [
            (
                Link {
                    resource: HeaderStr::Text(""),
                    ..plain
                },
                LinkError::Invalid("resource"),
            ),
            (
                Link {
                    resource: HeaderStr::Text(&a37),
                    ..plain
                },
                LinkError::Invalid("resource"),
            ),
            (
                Link {
                    res_id: Some(HeaderStr::Text(&a37)),
                    ..plain
                },
                LinkError::Invalid("resId"),
            ),
            (
                Link {
                    role: LinkRole::Other(HeaderStr::Text(&r33)),
                    ..plain
                },
                LinkError::Invalid("role"),
            ),
            (
                Link {
                    res_id: Some(HeaderStr::Text("a-b")),
                    ..sel
                },
                LinkError::Invalid("resId"),
            ),
            (
                Link {
                    action_res_id: Some(HeaderStr::Text(&a37)),
                    ..sel
                },
                LinkError::Invalid("actionResId"),
            ),
            (
                Link {
                    action_res_id: Some(HeaderStr::Text("P1")),
                    ..sel
                },
                LinkError::Invalid("actionResId"),
            ),
            (
                Link {
                    action_resource: Some(HeaderStr::Text(&a37)),
                    ..sel
                },
                LinkError::Invalid("actionResource"),
            ),
            (
                Link {
                    selected_property: Some(HeaderStr::Text("")),
                    ..sel
                },
                LinkError::Invalid("selectedProperty"),
            ),
            (
                Link {
                    confirm_icon: Some(ConfirmIcon::Unknown(HeaderStr::Text("stop"))),
                    ..plain
                },
                LinkError::Invalid("confirmIcon"),
            ),
            (
                Link {
                    selected_property: None,
                    ..sel
                },
                LinkError::Missing("selectedProperty"),
            ),
        ];
        for (i, (l, want)) in refused.iter().enumerate() {
            assert_eq!(l.write(&mut buf), Err(*want), "case {i}");
        }
        // The limits are written; a plain Link "resId" has the rules of REQ-PE-HDR-007 only.
        let ok = Link {
            resource: HeaderStr::Text(&a36),
            res_id: Some(HeaderStr::Text("A-1")),
            role: LinkRole::Other(HeaderStr::Text(&r32)),
            confirm_icon: Some(ConfirmIcon::Information),
            ..plain
        };
        let n = ok.write(&mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            std::format!(
                r#"{{"resource":"{a36}","resId":"A-1","role":"{r32}","confirmIcon":"information"}}"#
            )
            .as_bytes()
        );
        let ok = Link {
            action_res_id: Some(HeaderStr::Text(&a36)),
            action_resource: Some(HeaderStr::Text(&a36)),
            ..sel
        };
        assert!(ok.write(&mut buf).is_ok());
    }

    #[test]
    fn prop_parse_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..80))| {
            let _ = Link::parse(&bytes);
        });
    }
}
