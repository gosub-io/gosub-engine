#[cfg(test)]
mod tests;
mod v8;
pub use v8::*;

trait FromContext<T> {
    fn from_ctx(ctx: V8Context, value: T) -> Self;
}

trait IntoContext<T> {
    fn into_ctx(self, ctx: V8Context) -> T;
}

//impl into context for everything that implements FromContext
impl<T, U> IntoContext<U> for T
where
    U: FromContext<T>,
{
    fn into_ctx(self, ctx: V8Context) -> U {
        U::from_ctx(ctx, self)
    }
}
