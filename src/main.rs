use std::{
    collections::HashMap,
    io,
    process::{Command, Stdio},
};

use mdbook::{BookItem, errors::Result, preprocess::CmdPreprocessor};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Parser, Tag, TagEnd};
use tempfile::TempDir;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("supports") => {
            // Supports all renderers.
            return Ok(());
        }
        Some(arg) => {
            eprintln!("unknown argument: {arg}");
            std::process::exit(1);
        }
        None => {}
    }

    let (_ctx, mut book) = CmdPreprocessor::parse_input(io::stdin().lock())?;
    book.for_each_mut(|item| {
        let BookItem::Chapter(chapter) = item else {
            return;
        };
        match run_examples(&chapter.content) {
            Ok(new_content) => chapter.content = new_content,
            Err(e) => eprintln!("could not process chapter: {e}"),
        }
    });

    serde_json::to_writer(io::stdout().lock(), &book)?;

    Ok(())
}

fn run_examples(content: &str) -> Result<String> {
    let mut buf = String::with_capacity(content.len());

    let events = Rewriter::new(Parser::new(content));

    pulldown_cmark_to_cmark::cmark(events, &mut buf)?;
    Ok(buf)
}

struct Rewriter<'input> {
    parser: Parser<'input>,
    example_started: Option<String>,
    example_content: Option<CowStr<'input>>,
    output: Option<String>,
    example_dirs: HashMap<String, TempDir>,
}

impl<'input> Rewriter<'input> {
    fn new(parser: Parser<'input>) -> Self {
        Self {
            parser,
            example_started: None,
            example_content: None,
            output: None,
            example_dirs: HashMap::new(),
        }
    }
}

impl<'input> Iterator for Rewriter<'input> {
    type Item = Event<'input>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(output) = self.output.take() {
            return Some(Event::Html(output.into()));
        }

        let mut event = self.parser.next()?;
        match &mut event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ident))) => {
                if let Some(("bash", example)) = ident.split_once(',') {
                    self.example_started = Some(String::from(example));
                }
            }
            Event::Text(text) => {
                if self.example_started.is_some() {
                    self.example_content = Some(text.clone());
                    *text = text
                        .lines()
                        .filter(|line| !line.starts_with('$'))
                        .fold(String::new(), |mut acc, val| {
                            acc.push_str(val);
                            acc.push('\n');
                            acc
                        })
                        .into();
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let (Some(example), Some(content)) =
                    (self.example_started.take(), self.example_content.take())
                {
                    let mut saved_output = String::from("\n<pre><code>");
                    let dir = self
                        .example_dirs
                        .entry(example.clone())
                        .or_insert_with(|| TempDir::with_prefix(example).unwrap());

                    for mut command in content.lines() {
                        let silent = if let Some(c) = command.strip_prefix('$') {
                            command = c;
                            true
                        } else {
                            false
                        };

                        let output_config = || {
                            if silent {
                                Stdio::null()
                            } else {
                                Stdio::piped()
                            }
                        };

                        let output = Command::new("bash")
                            .current_dir(dir.path())
                            .arg("-c")
                            .arg(command)
                            .stdout(output_config())
                            .stderr(output_config())
                            .output()
                            .unwrap();
                        let get =
                            |out| ansi_to_html::convert(&String::from_utf8(out).unwrap()).unwrap();

                        if !silent {
                            let stdout = get(output.stdout);
                            let stderr = get(output.stderr);

                            saved_output.push_str("$ ");
                            saved_output.push_str(command);
                            saved_output.push('\n');
                            saved_output.push_str(&stdout);
                            saved_output.push_str(&stderr);
                            if !stdout.is_empty() || !stderr.is_empty() {
                                saved_output.push('\n');
                            }
                        }
                    }

                    saved_output.push_str("</code></pre>");
                    self.output = Some(saved_output);
                }
            }
            _ => (),
        }
        Some(event)
    }
}
