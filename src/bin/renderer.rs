use clap::ArgAction;
use url::Url;

use gosub_renderer::render_tree::TreeDrawer;
use gosub_shared::types::Result;
use gosub_styling::render_tree::RenderTree;
use gosub_taffy::TaffyLayouter;
use gosub_useragent::application::Application;
use gosub_vello::VelloBackend;

type Backend = VelloBackend;
type Layouter = TaffyLayouter;
type Drawer = TreeDrawer<Backend, Layouter>;
type Tree = RenderTree<Backend, Layouter>;

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

    application.start()?;

    Ok(())
}
