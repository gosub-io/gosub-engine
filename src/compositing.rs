use crate::layout::LayoutSize;
use crate::types::Result;

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/third_party/webrender/webrender_api/src/display_list.rs#L112
pub struct BuiltDisplayList;

impl BuiltDisplayList {
    pub fn into_data(self) -> (Vec<u8>, BuiltDisplayListDescriptor) {
        (Vec::new(), BuiltDisplayListDescriptor)
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/third_party/webrender/webrender_api/src/display_list.rs#L136
pub struct BuiltDisplayListDescriptor;

// See ipc_channel crate
pub struct IpcSender;

impl IpcSender {
    pub fn send(&self, _message: ScriptToCompositorMsg) -> Result<()> {
        todo!()
    }
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/shared/script/compositor.rs#L216
pub struct CompositorDisplayListInfo {
    pub content_size: LayoutSize,
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/shared/script/lib.rs#L1119
pub enum ScriptToCompositorMsg {
    SendDisplayList {
        display_list_info: CompositorDisplayListInfo,
        display_list_descriptor: BuiltDisplayListDescriptor,
    },
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/shared/compositing/lib.rs#L182
pub enum ForwardedToCompositorMsg {
    Layout(ScriptToCompositorMsg),
}

// See https://github.com/servo/servo/blob/5d7ed76b79de359ef1de2bdee83b32bd497d7cd8/components/compositing/compositor.rs#L128
pub struct IOCompositor;

impl IOCompositor {
    #[allow(unused)]
    fn handle_webrender_message(&self, _message: ForwardedToCompositorMsg) {
        todo!()
    }
}
