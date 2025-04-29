#[cfg(feature = "CFEngine")]
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::Write;
use std::mem;
use std::str;
use std::vec;

use crate::compiler::Compiler;
// for bug!
use crate::parser::Token;
use log::{error, log};
use serde::Serialize;

use super::{Context, Data, Error, Result, to_data};

/// `Template` represents a compiled mustache file.
#[derive(Debug, Clone)]
pub struct Template {
    ctx: Context,
    tokens: Vec<Token>,
    partials: HashMap<String, Vec<Token>>,
}

/// Construct a `Template`. This is not part of the impl of Template so it is
/// not exported outside of mustache.
pub fn new(ctx: Context, tokens: Vec<Token>, partials: HashMap<String, Vec<Token>>) -> Template {
    Template {
        ctx,
        tokens,
        partials,
    }
}

impl Template {
    /// Renders the template with the `Encodable` data.
    pub fn render<W, T>(&self, wr: &mut W, data: &T) -> Result<()>
    where
        W: Write,
        T: Serialize,
    {
        let data = to_data(data)?;
        self.render_data(wr, &data)
    }

    /// Renders the template with the `Data`.
    pub fn render_data<W: Write>(&self, wr: &mut W, data: &Data) -> Result<()> {
        let mut render_ctx = RenderContext::new(self);
        let mut stack = vec![data];

        render_ctx.render(wr, &mut stack, &self.tokens)
    }

    /// Renders the template to a `String` with the `Encodable` data.
    pub fn render_to_string<T: Serialize>(&self, data: &T) -> Result<String> {
        let mut output = Vec::new();
        self.render(&mut output, data)?;
        String::from_utf8(output).map_err(|_| Error::InvalidStr)
    }

    /// Renders the template to a `String` with the `Data`.
    pub fn render_data_to_string(&self, data: &Data) -> Result<String> {
        let mut output = Vec::new();
        self.render_data(&mut output, data)?;
        String::from_utf8(output).map_err(|_| Error::InvalidStr)
    }
}

struct RenderContext<'a> {
    template: &'a Template,
    indent: String,
    line_start: bool,
    at: String,
}

impl<'a> RenderContext<'a> {
    fn new(template: &'a Template) -> RenderContext<'a> {
        RenderContext {
            template,
            indent: "".to_string(),
            line_start: true,
            at: "".to_string(),
        }
    }

    fn render<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        tokens: &[Token],
    ) -> Result<()> {
        for token in tokens.iter() {
            self.render_token(wr, stack, token)?;
        }

        Ok(())
    }

    fn render_token<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        token: &Token,
    ) -> Result<()> {
        match *token {
            #[cfg(feature = "CFEngine")]
            Token::At => self.render_at(wr),
            #[cfg(feature = "CFEngine")]
            Token::JSON(ref path, _) => self.render_json(wr, stack, path, false),
            #[cfg(feature = "CFEngine")]
            Token::JSONMulti(ref path, _) => self.render_json(wr, stack, path, true),
            #[cfg(feature = "CFEngine")]
            Token::TopJSON(ref path, _) => self.render_json(wr, stack, path, false),
            #[cfg(feature = "CFEngine")]
            Token::TopJSONMulti(ref path, _) => self.render_json(wr, stack, path, true),
            #[cfg(feature = "CFEngine")]
            Token::TopSection(ref children) => self.render_section_top(wr, stack, children),
            Token::Text(ref value) => self.render_text(wr, value),
            Token::EscapedTag(ref path, _) => self.render_etag(wr, stack, path),
            Token::UnescapedTag(ref path, _) => self.render_utag(wr, stack, path),
            Token::Section(ref path, true, ref children, _, _, _) => {
                self.render_inverted_section(wr, stack, path, children)
            }
            Token::Section(ref path, false, ref children, _, _, ref fdata) => {
                self.render_section(wr, stack, path, children, fdata)
            }
            Token::Partial(ref name, ref indent, _) => self.render_partial(wr, stack, name, indent),
            Token::IncompleteSection(..) => {
                bug!("render_token should not encounter IncompleteSections");
                Err(Error::IncompleteSection)
            }
        }
    }

    fn write_tracking_newlines<W: Write>(&mut self, wr: &mut W, value: &str) -> Result<()> {
        wr.write_all(value.as_bytes())?;
        self.line_start = match value.chars().last() {
            None => self.line_start, // None == ""
            Some('\n') => true,
            _ => false,
        };

        Ok(())
    }

    fn write_indent<W: Write>(&mut self, wr: &mut W) -> Result<()> {
        if self.line_start {
            wr.write_all(self.indent.as_bytes())?;
        }

        Ok(())
    }

    #[cfg(feature = "CFEngine")]
    fn render_at<W: Write>(&mut self, wr: &mut W) -> Result<()> {
        if !self.at.is_empty() {
            let at = self.at.clone();
            self.write_tracking_newlines(wr, &at)?;
        }
        Ok(())
    }

    fn render_text<W: Write>(&mut self, wr: &mut W, value: &str) -> Result<()> {
        // Indent the lines.
        if self.indent.is_empty() {
            return self.write_tracking_newlines(wr, value);
        } else {
            let mut pos = 0;
            let len = value.len();

            while pos < len {
                let v = &value[pos..];
                let line = match v.find('\n') {
                    None => {
                        let line = v;
                        pos = len;
                        line
                    }
                    Some(i) => {
                        let line = &v[..i + 1];
                        pos += i + 1;
                        line
                    }
                };

                if line.as_bytes()[0] != b'\n' {
                    self.write_indent(wr)?;
                }

                self.write_tracking_newlines(wr, line)?;
            }
        }

        Ok(())
    }

    fn render_etag<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        path: &[String],
    ) -> Result<()> {
        let mut bytes = vec![];

        self.render_utag(&mut bytes, stack, path)?;

        for b in bytes {
            match b {
                b'<' => wr.write_all(b"&lt;")?,
                b'>' => wr.write_all(b"&gt;")?,
                b'&' => wr.write_all(b"&amp;")?,
                b'"' => wr.write_all(b"&quot;")?,
                b'\'' => wr.write_all(b"&#39;")?,
                _ => wr.write_all(&[b])?,
            }
        }

        Ok(())
    }

    fn render_utag<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        path: &[String],
    ) -> Result<()> {
        match self.find(path, stack) {
            None => {}
            Some(value) => {
                self.write_indent(wr)?;

                // Currently this doesn't allow Option<Option<Foo>>, which
                // would be un-nameable in the view anyway, so I'm unsure if it's
                // a real problem. Having {{foo}} render only when `foo = Some(Some(val))`
                // seems unintuitive and may be surprising in practice.
                if let Data::Null = *value {
                    return Ok(());
                }

                match *value {
                    Data::String(ref value) => {
                        self.write_tracking_newlines(wr, value)?;
                    }

                    // etags and utags use the default delimiter.
                    Data::Fun(ref fcell) => {
                        let f = &mut *fcell.borrow_mut();
                        let tokens = self.render_fun("", "{{", "}}", f)?;
                        self.render(wr, stack, &tokens)?;
                    }

                    Data::Bool(ref b) => {
                        self.write_tracking_newlines(wr, &b.to_string())?;
                    }

                    ref value => {
                        bug!("render_utag: unexpected value {:?}", value);
                    }
                }
            }
        };

        Ok(())
    }

    #[cfg(feature = "CFEngine")]
    fn write_tracking_newlines_json<T: serde::Serialize, W: Write>(
        &mut self,
        wr: &mut W,
        data: T,
        pretty: bool,
    ) -> Result<()> {
        let json = match pretty {
            true => serde_json::to_string_pretty(&data),
            false => serde_json::to_string(&data),
        };
        self.write_tracking_newlines(wr, &json.unwrap_or(String::new()))?;
        Ok(())
    }

    #[cfg(feature = "CFEngine")]
    fn render_json<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        path: &[String],
        pretty: bool,
    ) -> Result<()> {
        if path.first() == Some(&"-top-".to_string()) {
            if let Some(v) = stack.first() {
                self.write_tracking_newlines_json(wr, &v, pretty)?;
            }
        } else {
            match self.find(path, stack) {
                None => {}
                Some(value) => {
                    self.write_indent(wr)?;
                    match *value {
                        Data::Null => {}
                        Data::String(ref v) => {
                            self.write_tracking_newlines(wr, v)?;
                        }
                        Data::Bool(ref v) => {
                            self.write_tracking_newlines(wr, &v.to_string())?;
                        }
                        Data::Fun(ref fcell) => {
                            let f = &mut *fcell.borrow_mut();
                            let tokens = self.render_fun("", "{{", "}}", f)?;
                            self.render(wr, stack, &tokens)?;
                        }
                        Data::Vec(ref v) => {
                            self.write_tracking_newlines_json(wr, v, pretty)?;
                        }
                        Data::Map(ref v) => {
                            let v: BTreeMap<_, _> = v.into_iter().collect();
                            self.write_tracking_newlines_json(wr, &v, pretty)?;
                        }
                    }
                }
            };
        }
        Ok(())
    }

    fn render_inverted_section<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        path: &[String],
        children: &[Token],
    ) -> Result<()> {
        match self.find(path, stack) {
            None => {}
            Some(Data::Null) => {}
            Some(Data::Bool(false)) => {}
            Some(Data::Vec(xs)) if xs.is_empty() => {}
            Some(_) => {
                return Ok(());
            }
        }

        self.render(wr, stack, children)
    }

    #[cfg(feature = "CFEngine")]
    fn render_section_top<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        children: &[Token],
    ) -> Result<()> {
        let i = stack.clone();
        let nstack = i.iter();
        for v in nstack {
            match Some(v) {
                None => {}
                Some(value) => match *value {
                    Data::Map(m) => {
                        if children.contains(&Token::At) {
                            let b: BTreeMap<_, _> = m.into_iter().collect();
                            for (k, v) in b.iter() {
                                stack.push(v);
                                self.at = k.to_string();
                                self.render(wr, stack, children)?;
                                self.at = "".to_string();
                                stack.pop();
                            }
                        } else {
                            stack.push(value);
                            self.render(wr, stack, children)?;
                            stack.pop();
                        }
                    }
                    _ => {}
                },
            };
        }
        Ok(())
    }

    fn render_section<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        path: &[String],
        children: &[Token],
        fdata: &[String],
    ) -> Result<()> {
        match self.find(path, stack) {
            None => {}
            Some(value) => match value {
                Data::Null => {}
                Data::Bool(true) => self.render(wr, stack, children)?,
                Data::Bool(false) => (),
                Data::String(val) => {
                    if !val.is_empty() {
                        stack.push(value);
                        self.render(wr, stack, children)?;
                        stack.pop();
                    }
                }
                Data::Vec(vs) => {
                    for (i, v) in vs.iter().enumerate() {
                        stack.push(v);
                        self.at = i.to_string();
                        self.render(wr, stack, children)?;
                        self.at = "".to_string();
                        stack.pop();
                    }
                }
                Data::Map(_m) => {
                    #[cfg(feature = "CFEngine")]
                    if children.contains(&Token::At) {
                        let b: BTreeMap<_, _> = _m.into_iter().collect();
                        for (k, v) in b.iter() {
                            stack.push(v);
                            self.at = k.to_string();
                            self.render(wr, stack, children)?;
                            self.at = "".to_string();
                            stack.pop();
                        }
                    } else {
                        stack.push(value);
                        self.render(wr, stack, children)?;
                        stack.pop();
                    }

                    #[cfg(not(feature = "CFEngine"))]
                    {
                        stack.push(value);
                        self.render(wr, stack, children)?;
                        stack.pop();
                    }
                }
                Data::Fun(fcell) => {
                    let f = &mut *fcell.borrow_mut();
                    let tokens = self.render_fun(&fdata[1], &fdata[0], &fdata[2], f)?;
                    self.render(wr, stack, &tokens)?;
                }
            },
        };
        Ok(())
    }

    fn render_partial<W: Write>(
        &mut self,
        wr: &mut W,
        stack: &mut Vec<&Data>,
        name: &str,
        indent: &str,
    ) -> Result<()> {
        match self.template.partials.get(name) {
            None => (),
            Some(tokens) => {
                let mut indent = self.indent.clone() + indent;

                mem::swap(&mut self.indent, &mut indent);
                self.render(wr, stack, tokens)?;
                mem::swap(&mut self.indent, &mut indent);
            }
        };

        Ok(())
    }

    fn render_fun(
        &self,
        src: &str,
        otag: &str,
        ctag: &str,
        f: &mut Box<dyn FnMut(String) -> String + Send + 'static>,
    ) -> Result<Vec<Token>> {
        let src = f(src.to_string());

        let compiler = Compiler::new_with(
            self.template.ctx.clone(),
            src.chars(),
            self.template.partials.clone(),
            otag.to_string(),
            ctag.to_string(),
        );

        let (tokens, _) = compiler.compile()?;
        Ok(tokens)
    }

    fn find<'c>(&self, path: &[String], stack: &mut Vec<&'c Data>) -> Option<&'c Data> {
        // If we have an empty path, we just want the top value in our stack.
        if path.is_empty() {
            match stack.last() {
                None => {
                    return None;
                }
                Some(data) => {
                    return Some(*data);
                }
            }
        }

        // Otherwise, find the stack that has the first part of our path.
        let mut value = None;

        for data in stack.iter().rev() {
            match **data {
                Data::Map(ref m) => {
                    if let Some(v) = m.get(&path[0]) {
                        value = Some(v);
                        break;
                    }
                }
                _ => { /* continue searching the stack */ }
            }
        }

        // Walk the rest of the path to find our final value.
        let mut value = match value {
            Some(value) => value,
            None => {
                return None;
            }
        };

        for part in path[1..].iter() {
            match *value {
                Data::Map(ref m) => match m.get(part) {
                    Some(v) => {
                        value = v;
                    }
                    None => {
                        return None;
                    }
                },
                _ => {
                    return None;
                }
            }
        }

        Some(value)
    }
}

#[cfg(feature = "CFEngine")]
#[cfg(test)]
mod tests {
    use crate::compile_str;

    use super::*;

    fn render_data(template: &Template, data: &Data) -> String {
        let mut bytes = vec![];
        template
            .render_data(&mut bytes, data)
            .expect("Failed to render data");
        String::from_utf8(bytes).expect("Failed ot encode as String")
    }

    #[test]
    fn test_json_simple_string() {
        let template = compile_str("Hello, {{$name}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("name".to_string(), Data::String("Ferris".to_string()));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "Hello, Ferris".to_string()
        );
    }

    #[test]
    fn test_json_simple_vec() {
        let template = compile_str("{{$v}}").expect("failed to compile");
        let v = vec![
            Data::String("A".to_string()),
            Data::String("B".to_string()),
            Data::String("C".to_string()),
        ];
        let mut ctx = HashMap::new();
        ctx.insert("v".to_string(), Data::Vec(v));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "[\"A\",\"B\",\"C\"]".to_string()
        );
    }

    #[test]
    fn test_json_simple_map() {
        let template = compile_str("{{$v}}").expect("failed to compile");
        let mut v = HashMap::new();
        v.insert("k1".to_string(), Data::String("A".to_string()));
        v.insert("k2".to_string(), Data::String("B".to_string()));
        let mut ctx = HashMap::new();
        ctx.insert("v".to_string(), Data::Map(v));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\"k1\":\"A\",\"k2\":\"B\"}".to_string()
        );
    }

    #[test]
    fn test_json_bool() {
        let template = compile_str("{{$b}}").expect("failed to compile");
        let b = true;
        let mut ctx = HashMap::new();
        ctx.insert("b".to_string(), Data::Bool(b));
        assert_eq!(render_data(&template, &Data::Map(ctx)), "true".to_string());
    }

    #[test]
    fn test_bool() {
        let template = compile_str("{{b}}").expect("failed to compile");
        let b = true;
        let mut ctx = HashMap::new();
        ctx.insert("b".to_string(), Data::Bool(b));
        assert_eq!(render_data(&template, &Data::Map(ctx)), "true".to_string());
    }

    #[test]
    fn test_top_json() {
        let template = compile_str("{{$-top-}}").expect("failed to compile");
        let b = true;
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(b));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\"a\":\"String\",\"b\":true}".to_string()
        );
    }

    #[test]
    fn test_dot_json() {
        let template = compile_str("{{$.}}").expect("failed to compile");
        let b = true;
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(b));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\"a\":\"String\",\"b\":true}".to_string()
        );
    }

    #[test]
    fn test_top_json_multi() {
        let template = compile_str("{{%-top-}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(true));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\n  \"a\": \"String\",\n  \"b\": true\n}".to_string()
        );
    }

    #[test]
    fn test_dot_json_multi() {
        let template = compile_str("{{%.}}").expect("failed to compile");
        let b = true;
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(b));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\n  \"a\": \"String\",\n  \"b\": true\n}".to_string()
        );
    }

    #[test]
    fn test_section() {
        let template = compile_str("{{#a}}{{$.}} {{/a}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        let v = vec![
            Data::String("String1".to_string()),
            Data::String("String2".to_string()),
            Data::String("String3".to_string()),
        ];
        ctx.insert("a".to_string(), Data::Vec(v));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "String1 String2 String3 "
        );
    }

    #[test]
    fn test_top_section() {
        let template = compile_str("{{#-top-}}{{$.}}{{/-top-}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(true));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\"a\":\"String\",\"b\":true}".to_string()
        );
    }

    #[test]
    fn test_top_section_multi() {
        let template = compile_str("{{#-top-}}{{%.}}{{/-top-}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(true));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "{\n  \"a\": \"String\",\n  \"b\": true\n}".to_string()
        );
    }

    #[test]
    fn test_boolean_section() {
        let template = compile_str(
            "{{#bt}}This text is rendered!{{/bt}}{{#bf}}This text is NOT rendered!{{/bf}}",
        )
        .expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("bt".to_string(), Data::Bool(true));
        ctx.insert("bf".to_string(), Data::Bool(false));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "This text is rendered!".to_string()
        );
    }

    #[test]
    fn test_rendering_vec_map_top() {
        let t = "{{#bf}}This text is NOT rendered{{/bf}}{{#fruits}}- {{$.}}\n{{/fruits}}\n{{$m}}\n{{$m.key3}}\n{{%-top-}}\n{{#bt}}This text is rendered!{{/bt}}";
        let template = compile_str(t).expect("failed to compile");
        let mut ctx = HashMap::new();
        let v = vec![
            Data::String("Apple".to_string()),
            Data::String("Cherry".to_string()),
            Data::String("Orange".to_string()),
        ];
        let v2 = vec![
            Data::Bool(true),
            Data::String("String1".to_string()),
            Data::Bool(false),
        ];
        ctx.insert("fruits".to_string(), Data::Vec(v));

        let mut m = HashMap::new();
        m.insert("key1".to_string(), Data::String("Value1".to_string()));
        m.insert("key2".to_string(), Data::Bool(true));
        m.insert("key3".to_string(), Data::Vec(v2));
        ctx.insert("m".to_string(), Data::Map(m));
        ctx.insert("bt".to_string(), Data::Bool(true));
        ctx.insert("bf".to_string(), Data::Bool(false));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "- Apple\n- Cherry\n- Orange\n{\"key1\":\"Value1\",\"key2\":true,\"key3\":[true,\"String1\",false]}\n[true,\"String1\",false]\n{\n  \"bf\": false,\n  \"bt\": true,\n  \"fruits\": [\n    \"Apple\",\n    \"Cherry\",\n    \"Orange\"\n  ],\n  \"m\": {\n    \"key1\": \"Value1\",\n    \"key2\": true,\n    \"key3\": [\n      true,\n      \"String1\",\n      false\n    ]\n  }\n}\nThis text is rendered!"
        );
    }

    #[test]
    fn test_vec_at() {
        let template = compile_str("{{#v}}{{@}} {{/v}}").expect("failed to compile");
        let v = vec![
            Data::String("A".to_string()),
            Data::String("B".to_string()),
            Data::String("C".to_string()),
        ];
        let mut ctx = HashMap::new();
        ctx.insert("v".to_string(), Data::Vec(v));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "0 1 2 ".to_string()
        );
    }

    #[test]
    fn test_map_at() {
        let template = compile_str("{{#m}}{{@}} {{/m}}").expect("failed to compile");
        let mut m = HashMap::new();
        m.insert("key1".to_string(), Data::String("Value1".to_string()));
        m.insert("key2".to_string(), Data::Bool(true));
        m.insert("key3".to_string(), Data::String("Value3".to_string()));

        let mut ctx = HashMap::new();
        ctx.insert("m".to_string(), Data::Map(m));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "key1 key2 key3 ".to_string()
        );

        let template = compile_str("{{#m}}{{@}} {{@}} {{.}} {{/m}}").expect("failed to compile");
        let mut m = HashMap::new();
        m.insert("key1".to_string(), Data::String("Value1".to_string()));
        m.insert("key2".to_string(), Data::Bool(true));
        m.insert("key3".to_string(), Data::String("Value3".to_string()));

        let mut ctx = HashMap::new();
        ctx.insert("m".to_string(), Data::Map(m));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "key1 key1 Value1 key2 key2 true key3 key3 Value3 ".to_string()
        );
    }

    #[test]
    fn test_top_section_at() {
        let template = compile_str("{{#-top-}}{{@}} {{/-top-}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(true));
        assert_eq!(render_data(&template, &Data::Map(ctx)), "a b ".to_string());

        let template = compile_str("{{#-top-}}{{@}} {{.}} {{/-top-}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(true));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "a String b true ".to_string()
        );
    }

    #[test]
    fn test_top_in_top_section_at() {
        let template =
            compile_str("{{#-top-}}{{@}} {{$-top-}} {{/-top-}}").expect("failed to compile");
        let mut ctx = HashMap::new();
        ctx.insert("a".to_string(), Data::String("String".to_string()));
        ctx.insert("b".to_string(), Data::Bool(true));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "a {\"a\":\"String\",\"b\":true} b {\"a\":\"String\",\"b\":true} ".to_string()
        );
    }

    #[test]
    fn test_top_section_inside_vec_section() {
        let template = compile_str("{{#v}}{{@}} {{#-top-}}{{@}} {{$.}} {{/-top-}} {{/v}}")
            .expect("failed to compile");
        let v = vec![
            Data::String("A".to_string()),
            Data::String("B".to_string()),
            Data::String("C".to_string()),
        ];
        let mut ctx = HashMap::new();
        ctx.insert("v".to_string(), Data::Vec(v));
        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "0 v [\"A\",\"B\",\"C\"]  1 v [\"A\",\"B\",\"C\"]  2 v [\"A\",\"B\",\"C\"]  "
                .to_string()
        );
    }

    #[test]
    fn test_top_section_inside_map_section() {
        let template = compile_str("{{#m}}{{@}} {{#-top-}}{{@}} {{$.}} {{/-top-}} {{/m}}")
            .expect("failed to compile");
        let mut m = HashMap::new();
        m.insert("key1".to_string(), Data::String("Value1".to_string()));
        m.insert("key2".to_string(), Data::Bool(true));
        m.insert("key3".to_string(), Data::String("Value3".to_string()));
        let mut ctx = HashMap::new();
        ctx.insert("m".to_string(), Data::Map(m));
        ctx.insert(
            "s".to_string(),
            Data::String("This is a string".to_string()),
        );

        assert_eq!(
            render_data(&template, &Data::Map(ctx)),
            "key1 m {\"key1\":\"Value1\",\"key2\":true,\"key3\":\"Value3\"} s This is a string  key2 m {\"key1\":\"Value1\",\"key2\":true,\"key3\":\"Value3\"} s This is a string  key3 m {\"key1\":\"Value1\",\"key2\":true,\"key3\":\"Value3\"} s This is a string  ".to_string()
        );
    }
}
