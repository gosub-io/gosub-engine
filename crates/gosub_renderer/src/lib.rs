use std::cell::RefCell;
use std::rc::Rc;

use anyhow::anyhow;

use gosub_shared::types::Result;

use crate::render_tree::RenderTree;
use crate::window::WindowState;

mod window;

mod render;
pub mod render_tree;

pub struct Renderer<'a> {
    pub window: Option<WindowState<'a, Rc<RefCell<RenderTree>>>>,
}

impl<'a> Renderer<'a> {
    pub fn new(rt: Rc<RefCell<RenderTree>>) -> Result<Self> {
        let mut s = None;
        let window = WindowState::new(
            Box::new(move |scene, size, rt: &mut Rc<RefCell<RenderTree>>| {
                if Some(size) != s {
                    scene.reset();
                    s = Some(size);
                    let mut rt = rt.borrow_mut();
                    rt.render(scene, size);
                    true
                } else {
                    false
                }
            }),
            rt,
        )?;

        Ok(Self {
            window: Some(window),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        let window = self
            .window
            .take()
            .ok_or(anyhow!("Window already started"))?;
        window.start()
    }
}
