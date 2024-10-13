use std::sync::mpsc;
use std::{io, thread};

use clap::ArgAction;
use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_renderer::render_tree::TreeDrawer;
use gosub_rendering::render_tree::RenderTree;
use gosub_shared::types::Result;
use gosub_taffy::TaffyLayouter;
use gosub_useragent::application::{Application, CustomEventInternal, WindowOptions};
use gosub_vello::VelloBackend;
use url::Url;

type Backend = VelloBackend;
type Layouter = TaffyLayouter;

type CssSystem = Css3System;

type Document = DocumentImpl<CssSystem>;

type HtmlParser<'a> = Html5Parser<'a, Document, CssSystem>;

type Drawer = TreeDrawer<Backend, Layouter, Document, CssSystem>;
type Tree = RenderTree<Layouter, CssSystem>;

fn main() -> Result<()> {
    // simple_logger::init_with_level(log::Level::Info)?;

    let matches = clap::Command::new("Gosub Renderer")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .arg(
            clap::Arg::new("debug")
                .short('d')
                .long("debug")
                .action(ArgAction::SetTrue),
        )
        .get_matches();

    let url: String = matches.get_one::<String>("url").expect("url").to_string();
    let debug = matches.get_one::<bool>("debug").copied().unwrap_or(false);

    // let drawer: TreeDrawer<Tree, TaffyLayouter> = TreeDrawer::new(todo!(), TaffyLayouter, "".to_string().into(), debug);

    // let mut rt = load_html_rendertree(&url)?;
    //
    let mut application: Application<Drawer, Backend, Layouter, Tree, Document, CssSystem, HtmlParser> =
        Application::new(VelloBackend::new(), TaffyLayouter, debug);

    application.initial_tab(Url::parse(&url)?, WindowOptions::default());

    //this will initialize the application
    let p = application.proxy()?;

    thread::spawn(move || loop {
        let mut input = String::new();
        if let Err(e) = io::stdin().read_line(&mut input) {
            eprintln!("Error reading input: {e:?}");
            continue;
        };

        let input = input.trim();

        match input {
            "list" => {
                let (sender, receiver) = mpsc::channel();

                if let Err(e) = p.send_event(CustomEventInternal::SendNodes(sender)) {
                    eprintln!("Error sending event: {e:?}");
                    continue;
                }

                let node = match receiver.recv() {
                    Ok(node) => node,
                    Err(e) => {
                        eprintln!("Error receiving node: {e:?}");
                        continue;
                    }
                };

                println!("{}", node);
            }

            "unselect" => {
                if let Err(e) = p.send_event(CustomEventInternal::Unselect) {
                    eprintln!("Error sending event: {e:?}");
                }
            }

            _ => {}
        }

        if input.starts_with("select ") {
            let id = input.trim_start_matches("select ");
            let Ok(id) = id.parse::<u64>() else {
                eprintln!("Invalid id: {id}");
                continue;
            };

            if let Err(e) = p.send_event(CustomEventInternal::Select(id)) {
                eprintln!("Error sending event: {e:?}");
            }
        }
    });

    application.run()?;

    Ok(())
}
