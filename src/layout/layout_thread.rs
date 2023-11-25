use crate::html5::node::NodeId;
use crate::html5::parser::document::DocumentHandle;

use super::{BoxTree, DebugOptions, DisplayList, LayoutContext, WebrenderIpcSender};

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_thread_2020/lib.rs#L275
pub struct ScriptReflowResult;

pub struct ScriptReflowRequest {
    document: DocumentHandle,
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_thread/lib.rs#L113
pub struct LayoutThread {
    webrender_api: WebrenderIpcSender,
    debug: DebugOptions,
}

impl Default for LayoutThread {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutThread {
    pub fn new() -> Self {
        Self {
            webrender_api: WebrenderIpcSender::new(),
            debug: Default::default(),
        }
    }

    // See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_thread_2020/lib.rs#L792
    #[allow(unused)]
    fn handle_reflow(&self, data: &ScriptReflowRequest) -> ScriptReflowResult {
        let mut context = self.build_layout_context();
        let document = data.document.get();
        let root_node = document
            .get_node_by_id(NodeId::root())
            .expect("no root node");

        let box_tree = BoxTree::construct(&context, root_node);
        let fragment_tree = box_tree.layout();
        let mut display_list = DisplayList::new(fragment_tree.scrollable_overflow());

        let root_stacking_context =
            display_list.build_stacking_context_tree(&fragment_tree, &self.debug);
        display_list.build(&context, &fragment_tree, &root_stacking_context);

        let DisplayList {
            compositor_info,
            builder,
        } = display_list;

        self.webrender_api
            .send_display_list(compositor_info, builder.finalize());

        ScriptReflowResult
    }

    fn build_layout_context(&self) -> LayoutContext {
        LayoutContext
    }
}
