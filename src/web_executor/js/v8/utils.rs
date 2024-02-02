use crate::web_executor::js::v8::{V8Context, V8Function, V8FunctionCallBack};
use std::marker::PhantomData;
use std::mem::size_of;
use v8::{
    CallbackScope, FunctionCallback, FunctionCallbackArguments, FunctionCallbackInfo, ReturnValue,
};

// --- https://github.com/denoland/rusty_v8/blob/ff2a50ccdf7d5f7091e2bfbdedf0927101e2844c/src/support.rs#L562 ---
pub trait UnitType
where
    Self: Copy + Sized,
{
    #[inline(always)]
    fn get() -> Self {
        UnitValue::<Self>::get()
    }
}

impl<T> UnitType for T where T: Copy + Sized {}

#[derive(Copy, Clone, Debug)]
struct UnitValue<T>(PhantomData<T>)
where
    Self: Sized;

impl<T> UnitValue<T>
where
    Self: Copy + Sized,
{
    const SELF: Self = Self::new_checked();

    const fn new_checked() -> Self {
        // Statically assert that T is indeed a unit type.
        let size_must_be_0 = size_of::<T>();
        let s = Self(PhantomData::<T>);
        [s][size_must_be_0]
    }

    #[inline(always)]
    fn get_checked(self) -> T {
        // This run-time check serves just as a backup for the compile-time
        // check when Self::SELF is initialized.
        assert_eq!(size_of::<T>(), 0);
        unsafe { std::mem::MaybeUninit::<T>::zeroed().assume_init() }
    }

    #[inline(always)]
    pub fn get() -> T {
        // Accessing the Self::SELF is necessary to make the compile-time type check
        // work.
        Self::SELF.get_checked()
    }
}

#[derive(Debug)]
pub struct DefaultTag;

#[derive(Debug)]
pub struct IdenticalConversionTag;

pub trait MapFnFrom<'a, F, Tag = DefaultTag>
where
    F: UnitType,
    Self: Sized,
{
    fn mapping(ctx: &V8Context<'a>) -> Self;

    #[inline(always)]
    fn map_fn_from(ctx: &V8Context<'a>, _: F) -> Self {
        Self::mapping(ctx)
    }
}

impl<'a, F> MapFnFrom<'a, F, IdenticalConversionTag> for F
where
    Self: UnitType,
{
    #[inline(always)]
    fn mapping(ctx: &V8Context<'a>) -> Self {
        Self::get()
    }
}

pub trait MapFnTo<'a, T, Tag = DefaultTag>
where
    Self: UnitType,
    T: Sized,
{
    fn mapping(ctx: &V8Context<'a>) -> T;

    #[inline(always)]
    fn map_fn_to(self, ctx: &V8Context<'a>) -> T {
        Self::mapping(ctx)
    }
}

impl<'a, F, T, Tag> MapFnTo<'a, T, Tag> for F
where
    Self: UnitType,
    T: MapFnFrom<'a, F, Tag>,
{
    #[inline(always)]
    fn mapping(ctx: &V8Context<'a>) -> T {
        T::map_fn_from(ctx, F::get())
    }
}

pub trait CFnFrom<F>
where
    Self: Sized,
    F: UnitType,
{
    fn mapping() -> Self;

    #[inline(always)]
    fn c_fn_from(_: F) -> Self {
        Self::mapping()
    }
}

macro_rules! impl_c_fn_from {
  ($($arg:ident: $ty:ident),*) => {
    impl<F, R, $($ty),*> CFnFrom<F> for extern "C" fn($($ty),*) -> R
    where
      F: UnitType + Fn($($ty),*) -> R,
    {
      #[inline(always)]
      fn mapping() -> Self {
        extern "C" fn c_fn<F, R, $($ty),*>($($arg: $ty),*) -> R
        where
          F: UnitType + Fn($($ty),*) -> R,
        {
          (F::get())($($arg),*)
        }
        c_fn::<F, R, $($ty),*>
      }
    }
  };
}

impl_c_fn_from!();
impl_c_fn_from!(a0: A0);
impl_c_fn_from!(a0: A0, a1: A1);
impl_c_fn_from!(a0: A0, a1: A1, a2: A2);
impl_c_fn_from!(a0: A0, a1: A1, a2: A2, a3: A3);
impl_c_fn_from!(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4);
impl_c_fn_from!(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5);
impl_c_fn_from!(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6);

pub trait ToCFn<T>
where
    Self: UnitType,
    T: Sized,
{
    fn mapping() -> T;

    #[inline(always)]
    fn to_c_fn(self) -> T {
        Self::mapping()
    }
}

impl<F, T> ToCFn<T> for F
where
    Self: UnitType,
    T: CFnFrom<F>,
{
    #[inline(always)]
    fn mapping() -> T {
        T::c_fn_from(F::get())
    }
}

// --- copy end ---

impl<'a, F> MapFnFrom<'a, F> for FunctionCallback
where
    F: UnitType + Fn(&mut V8FunctionCallBack<'a>),
{
    fn mapping(ctx: &V8Context<'a>) -> Self {
        let f = |info: *const FunctionCallbackInfo| {
            let info = unsafe { &*info };
            let scope = &mut unsafe { CallbackScope::new(info) };
            let args = FunctionCallbackArguments::from_function_callback_info(info);
            let rv = ReturnValue::from_function_callback_info(info);

            // V8Function::callback(ctx, scope, args, rv, F::get());
        };
        f.to_c_fn()
    }
}
