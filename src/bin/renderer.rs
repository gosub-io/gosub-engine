use std::sync::mpsc;
use std::{io, thread};

use clap::ArgAction;
use url::Url;

use gosub_renderer::render_tree::TreeDrawer;
use gosub_shared::types::Result;
use gosub_styling::render_tree::RenderTree;
use gosub_taffy::TaffyLayouter;
use gosub_useragent::application::{Application, CustomEvent};
use gosub_vello::VelloBackend;

type Backend = VelloBackend;
type Layouter = TaffyLayouter;
type Drawer = TreeDrawer<Backend, Layouter>;
type Tree = RenderTree<Layouter>;

fn main() -> Result<()> {
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
    let mut application: Application<Drawer, Backend, Layouter, Tree> =
        Application::new(VelloBackend::new(), TaffyLayouter, debug);

    application.initial_tab(Url::parse(&url)?);

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

                if let Err(e) = p.send_event(CustomEvent::SendNodes(sender)) {
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
                if let Err(e) = p.send_event(CustomEvent::Unselect) {
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

            if let Err(e) = p.send_event(CustomEvent::Select(id)) {
                eprintln!("Error sending event: {e:?}");
            }
        }
    });

    application.run()?;

    Ok(())
}
