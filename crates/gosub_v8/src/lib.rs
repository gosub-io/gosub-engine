/// Opens a handle scope on the context's isolate, enters the context and binds a
/// `&mut v8::PinScope` to `$scope`. v8 scopes are pinned to the stack, so this cannot be
/// a function that returns one.
macro_rules! ctx_scope {
    ($scope:ident, $ctx:expr) => {
        let (mut isolate, context) = $ctx.scope_parts();
        v8::scope_with_context!($scope, &mut *isolate, &context);
        let $scope: &mut v8::PinScope<'_, '_> = $scope;
    };
}

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
