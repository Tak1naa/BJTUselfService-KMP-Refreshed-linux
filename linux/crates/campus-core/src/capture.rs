//! Conservative local HTML structure capture. All free text and active content are removed.
use scraper::{Html, Node};
pub fn redact(html: &str) -> String {
    let mut doc = Html::parse_document(html);
    let ids: Vec<_> = doc.tree.nodes().map(|n| n.id()).collect();
    let week = regex::Regex::new(r"^第?[0-9,\-\s]+周$").unwrap();
    let title = regex::Regex::new(r"^星期[一二三四五六日天1-7]\s*第[1-7]节$").unwrap();
    let css =
        regex::Regex::new(r"(?i)(?:background-color:\s*#[a-f0-9]{3,6}|max-width:\s*[0-9]+px)")
            .unwrap();
    for id in ids {
        let active = doc
            .tree
            .get(id)
            .and_then(|n| n.parent())
            .and_then(|p| p.value().as_element())
            .is_some_and(|e| matches!(e.name(), "script" | "style"));
        let mut node = doc.tree.get_mut(id).unwrap();
        match node.value() {
            Node::Text(t) => {
                let s = t.text.trim();
                let safe = if active {
                    String::new()
                } else if week.is_match(s)
                    || [
                        "课程性质",
                        "必修",
                        "限选",
                        "任选",
                        "体育",
                        "身份",
                        "部门",
                        "序号",
                    ]
                    .contains(&s)
                {
                    s.to_string()
                } else if s.starts_with("身份：") {
                    "身份：测试身份".into()
                } else if s.starts_with("部门：") {
                    "部门：测试学院".into()
                } else if s.is_empty() {
                    String::new()
                } else {
                    "测试数据".into()
                };
                t.text = safe.into();
            }
            Node::Comment(c) => c.comment = "".into(),
            Node::Element(e) => {
                e.attrs.retain(|(name, _)| {
                    [
                        "class", "id", "name", "type", "style", "title", "rowspan", "colspan",
                        "selected", "value",
                    ]
                    .contains(&name.local.as_ref())
                });
                for (name, value) in &mut e.attrs {
                    let s = value.as_ref();
                    *value = match name.local.as_ref() {
                        "value" => "fixture".into(),
                        "class" | "id" | "name" | "type"
                            if [
                                "name_right",
                                "nr_con",
                                "text-muted",
                                "loginForm",
                                "redirect",
                                "flowId",
                                "username",
                                "password",
                                "captcha",
                                "hidJson",
                                "zxjxjhh",
                                "text",
                                "hidden",
                                "checkbox",
                                "submit",
                            ]
                            .contains(&s) =>
                        {
                            s.to_string().into()
                        }
                        "style" => css
                            .find_iter(s)
                            .map(|m| m.as_str())
                            .collect::<Vec<_>>()
                            .join(";")
                            .into(),
                        "title" if title.is_match(s) => s.to_string().into(),
                        "rowspan" | "colspan"
                            if s.parse::<u8>().is_ok_and(|v| (1..=100).contains(&v)) =>
                        {
                            s.to_string().into()
                        }
                        "selected" => "".into(),
                        _ => "fixture".into(),
                    };
                }
            }
            _ => {}
        }
    }
    doc.html()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_free_text_urls_and_session_fields() {
        let s = redact(
            "<form id=loginForm><input name=flowId value=private-flow></form><script>var token='private-token'</script><a href='https://private.example/?session=private-session'>private-name</a><div style='background-image:url(private-url)'>private-name</div><!--private-comment-->",
        );
        for secret in [
            "private-flow",
            "private-token",
            "private-session",
            "private-name",
            "private-url",
            "private-comment",
        ] {
            assert!(!s.contains(secret));
        }
        assert!(s.contains("loginForm"));
    }
}
