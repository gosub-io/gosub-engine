use crate::traits::css3::CssSystem;
use crate::traits::document::Document;
use std::cell::{Ref, RefCell, RefMut};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::rc::Rc;

pub struct DocumentHandle<D: Document<C>, C: CssSystem>(pub Rc<RefCell<D>>, pub PhantomData<C>);

impl<C, D> DocumentHandle<D, C>
where
    C: CssSystem,
    D: Document<C>,
{
    /// Create a new DocumentHandle from a document
    pub fn create(document: D) -> Self {
        DocumentHandle(Rc::new(RefCell::new(document)), PhantomData)
    }

    /// Returns the document as referenced by the handle
    pub fn get(&self) -> Ref<D> {
        self.0.borrow()
    }

    /// Returns a
    pub fn get_mut(&mut self) -> RefMut<D> {
        self.0.borrow_mut()
    }
}

impl<C: CssSystem, D: Document<C> + Debug> Debug for DocumentHandle<D, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0.borrow())
    }
}

// impl<D: Document + PartialEq> PartialEq for DocumentHandle<D> {
//     fn eq(&self, other: &Self) -> bool {
//         self.0.eq(&other.0)
//     }
// }

impl<C: CssSystem, D: Document<C> + PartialEq> PartialEq for DocumentHandle<D, C> {
    fn eq(&self, other: &Self) -> bool {
        self.0.borrow().eq(&other.0.borrow())
    }
}

impl<C: CssSystem, D: Document<C> + Eq> Eq for DocumentHandle<D, C> {}

// NOTE: it is preferred to use Document::clone() when
// copying a DocumentHandle reference. However, for
// any structs using this handle that use #[derive(Clone)],
// this implementation is required.
impl<C: CssSystem, D: Document<C>> Clone for DocumentHandle<D, C> {
    fn clone(&self) -> DocumentHandle<D, C> {
        DocumentHandle(Rc::clone(&self.0), PhantomData)
    }
}
