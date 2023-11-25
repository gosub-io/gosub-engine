pub mod layout_thread;

use crate::{
    compositing::{BuiltDisplayList, CompositorDisplayListInfo, IpcSender, ScriptToCompositorMsg},
    html5::node::Node,
};
use log::warn;
use std::sync::mpsc::channel;

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/config/opts.rs#L149
#[derive(Default)]
pub struct DebugOptions;

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/shared/script/lib.rs#L1150
struct WebrenderIpcSender(IpcSender);

impl WebrenderIpcSender {
    fn new() -> Self {
        Self(IpcSender)
    }

    fn send_display_list(
        &self,
        display_list_info: CompositorDisplayListInfo,
        list: BuiltDisplayList,
    ) {
        let (display_list_data, display_list_descriptor) = list.into_data();
        // TODO: Set up channel somewhere else
        let (tx, _rx) = channel();

        if let Err(err) = self.0.send(ScriptToCompositorMsg::SendDisplayList {
            display_list_info,
            display_list_descriptor,
        }) {
            warn!("error sending display list: {err}");
        }

        if let Err(err) = tx.send(display_list_data) {
            warn!("error sending display data: {err}");
        }
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/third_party/webrender/webrender_api/src/display_list.rs#L1012
struct WebrenderDisplayListBuilder;

impl WebrenderDisplayListBuilder {
    fn finalize(self) -> BuiltDisplayList {
        BuiltDisplayList
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_2020/fragment_tree/fragment_tree.rs#L22
#[derive(Default)]
struct FragmentTree;

impl FragmentTree {
    fn scrollable_overflow(&self) -> LayoutSize {
        LayoutSize
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_2020/flow/root.rs#L32
struct BoxTree;

impl BoxTree {
    fn construct(_context: &LayoutContext, _node: &Node) -> Self {
        Self
    }

    fn layout(&self) -> FragmentTree {
        todo!()
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/third_party/webrender/webrender_api/src/units.rs#L91
pub struct LayoutSize;

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_2020/display_list/mod.rs#L57
pub struct DisplayList {
    compositor_info: CompositorDisplayListInfo,
    builder: WebrenderDisplayListBuilder,
}

impl DisplayList {
    fn new(content_size: LayoutSize) -> Self {
        Self {
            compositor_info: CompositorDisplayListInfo { content_size },
            builder: WebrenderDisplayListBuilder,
        }
    }

    fn build_stacking_context_tree(
        &self,
        _fragment_tree: &FragmentTree,
        _debug: &DebugOptions,
    ) -> StackingContext {
        StackingContext
    }

    fn build(
        &self,
        _context: &LayoutContext,
        _fratment_tree: &FragmentTree,
        _stacking_context: &StackingContext,
    ) {
        todo!()
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_2020/display_list/stacking_context.rs#L269
pub struct StackingContext;

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/shared/script_layout/message.rs#L122
#[derive(Debug, PartialEq)]
pub enum ReflowGoal {
    Full,
    TickAnimations,
    LayoutQuery,
    UpdateScrollNode,
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/layout_2020/context.rs#L23
pub struct LayoutContext;
