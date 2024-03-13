mod v8;

pub use v8::*;

trait FromContext<'a, T> {
    fn from_ctx(ctx: V8Context<'a>, value: T) -> Self;
}

trait IntoContext<'a, T> {
    fn into_ctx(self, ctx: V8Context<'a>) -> T;
}

//impl into context for everything that implements FromContext
impl<'a, T, U> IntoContext<'a, U> for T
where
    U: FromContext<'a, T>,
{
    fn into_ctx(self, ctx: V8Context<'a>) -> U {
        U::from_ctx(ctx, self)
    }
}
