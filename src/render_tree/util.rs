use std::{cell::RefCell, rc::Rc};

use super::Node;

// TODO: we need a NodeHandle wrapper to clean up this borrow_mut() stuff

pub fn add_text_node(reference_element: &Rc<RefCell<Node>>, new_text: &Rc<RefCell<Node>>) {
    let mut mut_ref = reference_element.as_ref().borrow_mut();
    if let Some(parent) = &mut_ref.parent {
        parent.as_ref().borrow_mut().add_child(&Rc::clone(new_text));
    } else {
        mut_ref.add_child(&Rc::clone(new_text));
    }
}
