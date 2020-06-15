use regex::Regex;
use select::document::Document;
use select::node::Node;
use select::predicate::{Attr, Class};
use tokio::fs::File;
use urlencoding::decode;

#[derive(Debug)]
struct WikiEntry<'a> {
    n: u32,
    title: String,
    categories: Vec<String>,
    nodes: Vec<Node<'a>>,
}

impl<'a> WikiEntry<'a> {
    fn file_name(&self) -> String {
        self.prefix(".txt")
    }

    fn short_prefix(&self, txt: &str) -> String {
        format!("vwt-{}{}", self.n, txt)
    }

    fn prefix(&self, txt: &str) -> String {
        format!("vim-wiki-tips-{}{}", &self.n, txt)
    }

    fn parse_node(&self, node: Node) -> String {
        match node.name() {
            Some("a") => self.parse_link(node),
            Some("hr") => format!("\n\n{}", "=".repeat(78)),
            Some("p") => {
                let mut text = String::new();
                for child in node.children() {
                    text.push_str(&self.parse_node(child));
                }
                let mut new_text = String::with_capacity(text.len());
                let mut col = 0;

                for word in text.trim().split_whitespace() {
                    col += word.chars().count() + 1;
                    if col > 78 {
                        new_text.push('\n');
                        col = word.chars().count();
                    };

                    new_text.push_str(word);
                    new_text.push(' ');
                }

                format!("\n\n{}", new_text.trim())
            }
            Some("pre") => format!(
                "\n\n>\n    {}\n<",
                node.text()
                    .trim()
                    .split('\n')
                    .collect::<Vec<&str>>()
                    .as_slice()
                    .join("\n    ")
            ),
            Some("h2") | Some("h3") => {
                let mut inner = String::new();
                for child in node.children() {
                    inner.push_str(&self.parse_node(child));
                }
                let inner = inner.trim().to_uppercase();
                let tag =
                    self.short_prefix(&format!("-{}", inner.to_lowercase().replace(" ", "-")));
                format!("\n\n{}  *{}*", inner, tag)
            }
            Some("li") | Some("div") | Some("b") | Some("i") | Some("span") => {
                node.children().map(|n| self.parse_node(n)).collect()
            }
            Some("code") => format!(
                "{}",
                node.children()
                    .map(|n| self.parse_node(n))
                    .collect::<String>()
            ),
            Some("ul") => {
                let mut res = String::new();
                for child in node.children() {
                    res.push_str(&format!("    - {}\n", self.parse_node(child)));
                }
                format!("\n{}", res)
            }
            Some("dl") => format!("\n\n{}", node.text().trim()),
            None => node.text().replace("\n", "").to_owned(),
            _ => String::new(),
        }
    }

    fn parse_link(&self, a_node: Node) -> String {
        let href = a_node.attr("href");

        if href.is_none() {
            return String::new();
        }

        let href = href.unwrap();

        // link to current page, replace with tag vwt-1-link
        if href.starts_with("#") {
            let prepared = format!(
                "-{}",
                href.replace("#", "").to_lowercase().replace("_", "-")
            );
            format!("{} |{}|", a_node.text(), self.short_prefix(&prepared))
        } else if href.starts_with("/wiki/VimTip") {
            // link to other vim tip
            format!(
                "{} |{}|",
                a_node.text(),
                href.replace("/wiki/VimTip", "vwt-")
            )
        } else if href.contains("vimdoc") {
            // link to vim docs
            let text = a_node.text();
            let text = text.trim();
            let re = Regex::new(r"'*'").unwrap();

            if re.is_match(text) {
                // check if link contains text 'sometext'
                text.to_owned()
            } else {
                let mut tag = decode(href.splitn(2, "tag=").last().unwrap()).unwrap();
                if tag == "*" {
                    tag = "star".to_owned();
                }
                format!("{} |{}|", text.replace(&tag, ""), tag)
            }
        } else if href.contains("printable=yes") // skip irrelevant links
            || href.contains("useskin=monobook")
            || href.contains("action=")
        {
            String::new()
        } else {
            let link = if href.starts_with("/wiki/") {
                format!("https://vim.fandom.com{}", href)
            } else {
                href.to_owned()
            };
            format!("{} [{}]", a_node.text(), link)
        }
    }

    fn to_vim_help(&self) -> String {
        let mut result = String::new();

        // add first row
        result.push_str(&format!(
            "*{}*   {} *{}*\n\n",
            self.file_name(),
            self.title,
            self.short_prefix("")
        ));

        for node in &self.nodes {
            result.push_str(&self.parse_node(*node));
        }

        return result;
    }

    fn parse(document: &'a Document, n: u32) -> Self {
        let title = document
            .find(Class("page-header__title"))
            .next()
            .unwrap()
            .text();

        let categories = document
            .find(Class("page-header__categories-links"))
            .into_selection()
            .children()
            .iter()
            .filter(|node| node.name() == Some("a"))
            .map(|node| node.text())
            .collect::<Vec<String>>();

        let mut entry = WikiEntry {
            n,
            nodes: vec![],
            title,
            categories,
        };

        for node in document
            .find(Attr("id", "mw-content-text"))
            .next()
            .unwrap()
            .children()
        {
            entry.nodes.push(node);
        }

        return entry;
    }

    async fn make_tip(n: u32) -> Result<(), Box<dyn std::error::Error>> {
        println!("Making tip {}", n);
        let mut original =
            tokio::fs::read_to_string(format!("originals/vim-wiki-tips-{}.html", n)).await;

        if original.is_err() {
            println!("File not found, downloading it ({})", n);
            let resp = reqwest::get(&format!("https://vim.fandom.com/wiki/VimTip{}", n)).await?;

            let text = resp.text().await?;
            original = Ok(text.clone());
            tokio::fs::write(
                format!("originals/vim-wiki-tips-{}.html", n),
                &text.into_bytes(),
            )
            .await?;
        }

        println!("Parsing tip {}", n);
        let document = Document::from(original.unwrap().as_str());
        let entry = WikiEntry::parse(&document, n);
        let result = entry.to_vim_help();

        tokio::fs::write(format!("doc/{}", entry.file_name()), &result.into_bytes()).await?;

        println!("Done tip {}", n);

        Ok(())
    }
}
#[tokio::main(core_threads = 16)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    for n in 1..15 {
        WikiEntry::make_tip(n).await?;
    }
    Ok(())
}
