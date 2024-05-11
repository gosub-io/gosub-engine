use std::cell::RefCell;
use std::rc::Rc;

use gosub_shared::types::Result;
use gosub_v8::V8Engine;
use gosub_webexecutor::js::{
    Args, IntoJSValue, IntoRustValue, JSContext, JSFunction, JSFunctionCallBack,
    JSFunctionCallBackVariadic, JSFunctionVariadic, JSGetterCallback, JSInterop, JSObject,
    JSRuntime, JSSetterCallback, JSValue, VariadicArgs, VariadicArgsInternal,
};
use gosub_webinterop::{web_fns, web_interop};

#[web_interop]
struct TestStruct {
    #[property]
    field: i32,
    #[property]
    field2: u32,
}

#[web_fns(1)]
impl TestStruct {
    fn add(&self, other: i32) -> i32 {
        self.field + other
    }

    fn add2(&mut self, other: i32) {
        self.field += other;
    }

    fn add3(a: i32, b: i32) -> i32 {
        a + b
    }

    fn vec_test(&self, vec: Vec<i32>) -> Vec<i32> {
        vec
    }

    fn slice_test<'a>(&self, slice: &'a [i32]) -> &'a [i32] {
        slice
    }

    fn tuple(&self, tuple: (i32, String)) -> (i32, String) {
        tuple
    }

    fn array_test(&self, array: &[[i32; 3]; 3]) {
        println!("{array:?}");
    }

    fn array_test2(&self, array: &[[i32; 3]; 3]) -> Vec<i32> {
        array.iter().flatten().copied().collect()
    }

    fn variadic2(args: &impl VariadicArgs) {
        for a in args.as_vec() {
            println!("got an arg...: {}", a.as_string().unwrap());
        }
    }

    fn variadic3(num: i32, args: &impl VariadicArgs) {
        println!("got num arg...: {num}");
        for a in args.as_vec() {
            println!("got an arg...: {}", a.as_string().unwrap());
        }
    }

    fn variadic3_2(num: i32, args: &impl VariadicArgs) -> Vec<i32> {
        let mut vec = Vec::new();
        vec.push(num);
        for a in args.as_vec() {
            vec.push(a.as_number().unwrap() as i32);
        }

        vec
    }

    fn variadic4<RT: JSRuntime>(_args: &RT::VariadicArgs) {}

    #[generic(I, i32, String)]
    fn variadic5<RT: JSRuntime, I: T1>(_i: I, _args: &RT::VariadicArgs, _ctx: &RT::Context) {}

    fn test222() {}

    fn uses_ctx(_ctx: &impl JSContext) {}

    #[generic(T, i32, String)]
    fn generic<T: T1>(_num: i32, _val: T) {}

    #[generic(T1, i32)]
    fn generic2(_num: u64, _val: impl T1) {}
}

trait T1 {}

impl T1 for i32 {}

impl T1 for String {}

#[test]
fn macro_interop() {
    let test_struct = TestStruct {
        field: 14,
        field2: 14,
    };

    let mut engine = V8Engine::new();
    let mut context = engine.new_context().unwrap();

    TestStruct::implement::<V8Engine>(Rc::new(RefCell::new(test_struct)), context.clone()).unwrap();

    let out = context
        .run(
            r#"
        let calls = []
        calls.push([TestStruct.add(3), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.add2(3), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.add3(3, 4), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.vec_test([1, 2, 3]), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.slice_test([1, 2, 3]), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.tuple([1, "2"]), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.array_test([[1, 2, 3], [4, 5, 6], [7, 8, 9]]), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.array_test2([[1, 2, 3], [4, 5, 6], [7, 8, 9]]), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.variadic2(1, 2, 3, 4, 5, 6, 7, 8, 9, 10), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.variadic3(1, 2, 3, 4, 5, 6, 7, 8, 9, 10), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.variadic3_2(1, 2, 3, 4, 5, 6, 7, 8, 9, 10), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.variadic4(1, 2, 3, 4, 5, 6, 7, 8, 9, 10), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.variadic5(1, 2, 3, 4, 5, 6, 7, 8, 9, 10), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.test222(), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.uses_ctx(), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.generic(1, "hello"), TestStruct.field, TestStruct.field2])
        calls.push([TestStruct.generic2(1, 2), TestStruct.field, TestStruct.field2])
        
        calls
        "#).expect("failed to run");

    let mut expected = vec![
        "17,14,14",
        ",17,14",
        "7,17,14",
        "1,2,3,17,14",
        "1,2,3,17,14",
        "1,2,17,14",
        ",17,14",
        "1,2,3,4,5,6,7,8,9,17,14",
        ",17,14",
        ",17,14",
        "1,2,3,4,5,6,7,8,9,10,17,14",
        ",17,14",
        ",17,14",
        ",17,14",
        ",17,14",
        ",17,14",
        ",17,14",
    ];

    let arr = out
        .as_array()
        .expect("failed to get array from run ret value");

    for v in arr {
        assert_eq!(v.as_string().unwrap(), expected.remove(0));
    }
}

#[derive(Debug)]
struct Test2 {
    field: i32,
    other_field: String,
}

impl Test2 {
    fn cool_fn(&self) -> i32 {
        self.field
    }

    fn add(&mut self, other: i32) {
        self.field += other;
    }

    fn concat(&self, other: String) -> String {
        self.other_field.clone() + &other
    }

    fn takes_ref(&self, other: &String) -> String {
        self.other_field.clone() + other
    }

    fn variadic<A: VariadicArgs>(&self, nums: &A) {
        for a in nums.as_vec() {
            println!("got an arg...: {}", a.as_string().unwrap());
        }
    }

    fn generic<A: I32, B: U64>(&self, val: A, _val2: B) -> A {
        val
    }
}

trait I32 {}

impl I32 for i32 {}

impl I32 for String {}

impl I32 for bool {}

trait U64 {}

impl U64 for u64 {}

impl U64 for String {}

impl JSInterop for Test2 {
    //this function will be generated by a macro
    fn implement<RT: JSRuntime>(s: Rc<RefCell<Self>>, mut ctx: RT::Context) -> Result<()> {
        let obj = ctx.new_global_object("test2")?; //#name

        {
            //field getter and setter
            let getter = {
                let s = Rc::clone(&s);
                Box::new(move |cb: &mut RT::GetterCB| {
                    let ctx = cb.context();
                    let value = s.borrow().field;
                    println!("got a call to getter: {value}");
                    let value = match value.to_js_value(ctx.clone()) {
                        Ok(value) => value,
                        Err(e) => {
                            cb.error(e);
                            return;
                        }
                    };
                    cb.ret(value);
                })
            };

            let setter = {
                let s = Rc::clone(&s);
                Box::new(move |cb: &mut RT::SetterCB| {
                    let value = cb.value();
                    let value = match value.as_number() {
                        Ok(value) => value,
                        Err(e) => {
                            cb.error(e);
                            return;
                        }
                    };

                    println!("got a call to setter: {value}");

                    s.borrow_mut().field = value as i32;
                })
            };

            obj.set_property_accessor("field", getter, setter)?;
        }

        {
            //other_field getter and setter
            let getter = {
                let s = Rc::clone(&s);
                Box::new(move |cb: &mut RT::GetterCB| {
                    let ctx = cb.context();
                    let value = s.borrow().other_field.clone();
                    println!("got a call to getter: {value}");
                    let value = match value.to_js_value(ctx.clone()) {
                        Ok(value) => value,
                        Err(e) => {
                            cb.error(e);
                            return;
                        }
                    };
                    cb.ret(value);
                })
            };

            let setter = {
                let s = Rc::clone(&s);
                Box::new(move |cb: &mut RT::SetterCB| {
                    let value = cb.value();
                    let value = match value.as_string() {
                        Ok(value) => value,
                        Err(e) => {
                            cb.error(e);
                            return;
                        }
                    };

                    println!("got a call to setter: {value}");

                    s.borrow_mut().other_field = value;
                })
            };

            obj.set_property_accessor("other_field", getter, setter)?;
        }

        let cool_fn = {
            let s = Rc::clone(&s);
            RT::Function::new(ctx.clone(), move |cb| {
                let num_args = 0; //function.arguments.len();
                if num_args != cb.len() {
                    cb.error("wrong number of arguments");
                    return;
                }

                let ctx = cb.context();

                let ret = match s.borrow().cool_fn().to_js_value(ctx) {
                    Ok(ret) => ret,
                    Err(e) => {
                        cb.error(e);
                        return;
                    }
                };

                cb.ret(ret);
            })?
        };

        obj.set_method("cool_fn", &cool_fn)?;

        let add = {
            let s = Rc::clone(&s);
            RT::Function::new(ctx.clone(), move |cb| {
                let num_args = 1; //function.arguments.len();
                if num_args != cb.len() {
                    cb.error("wrong number of arguments");
                    return;
                }

                let ctx = cb.context();

                let Some(arg0) = cb.args().get(0, ctx.clone()) else {
                    cb.error("failed to get argument");
                    return;
                };

                let Ok(arg0) = arg0.as_number() else {
                    cb.error("failed to convert argument");
                    return;
                };

                #[allow(clippy::unit_arg)]
                let ret = s
                    .borrow_mut()
                    .add(arg0 as i32)
                    .to_js_value(ctx)
                    .unwrap();

                cb.ret(ret);
            })?
        };
        obj.set_method("add", &add)?;

        let concat = {
            let s = Rc::clone(&s);
            RT::Function::new(ctx.clone(), move |cb| {
                let num_args = 1; //function.arguments.len();
                if num_args != cb.len() {
                    cb.error("wrong number of arguments");
                    return;
                }

                let ctx = cb.context();

                let Some(arg0) = cb.args().get(0, ctx.clone()) else {
                    cb.error("failed to get argument");
                    return;
                };

                let Ok(arg0) = arg0.to_rust_value() else {
                    cb.error("failed to convert argument");
                    return;
                };

                let ret = s.borrow().concat(arg0).to_js_value(ctx).unwrap();

                cb.ret(ret);
            })?
        };
        obj.set_method("concat", &concat)?;

        let takes_ref = {
            let s = Rc::clone(&s);
            RT::Function::new(ctx.clone(), move |cb| {
                let num_args = 1; //function.arguments.len();
                if num_args != cb.len() {
                    cb.error("wrong number of arguments");
                    return;
                }

                let ctx = cb.context();

                let Some(arg0) = cb.args().get(0, ctx.clone()) else {
                    cb.error("failed to get argument");
                    return;
                };

                let Ok(arg0) = arg0.to_rust_value() else {
                    cb.error("failed to convert argument");
                    return;
                };

                let ret = s
                    .borrow()
                    .takes_ref(&arg0)
                    .to_js_value(ctx)
                    .unwrap();

                cb.ret(ret);
            })?
        };
        obj.set_method("takes_ref", &takes_ref)?;

        let variadic = {
            let s = Rc::clone(&s);
            RT::FunctionVariadic::new(ctx.clone(), move |cb| {
                eprintln!("got a call to variadic");
                let ctx = cb.context();

                let args = cb.args().variadic(ctx.clone());

                #[allow(clippy::unit_arg)]
                let ret = s.borrow().variadic(&args).to_js_value(ctx).unwrap();

                cb.ret(ret);
            })?
        };

        obj.set_method_variadic("variadic", &variadic)?;

        let generic = {
            let s = Rc::clone(&s);
            RT::Function::new(ctx.clone(), move |cb| {
                let num_args = 1; //function.arguments.len();
                if num_args != cb.len() {
                    cb.error("wrong number of arguments");
                    return;
                }

                let ctx = cb.context();

                let Some(arg0) = cb.args().get(0, ctx.clone()) else {
                    cb.error("failed to get argument");
                    return;
                };

                let Some(arg1) = cb.args().get(1, ctx.clone()) else {
                    cb.error("failed to get argument");
                    return;
                };

                if arg0.is_number() {
                    let Ok(arg0): Result<i32> = arg0.to_rust_value() else {
                        cb.error("failed to convert argument");
                        return;
                    };

                    if arg1.is_number() {
                        let Ok(arg1): Result<u64> = arg1.to_rust_value() else {
                            cb.error("failed to convert argument");
                            return;
                        };

                        let ret = s
                            .borrow()
                            .generic(arg0, arg1)
                            .to_js_value(ctx.clone())
                            .unwrap();

                        cb.ret(ret);
                    }
                }

                if arg0.is_string() {
                    let Ok(arg0): Result<String> = arg0.to_rust_value() else {
                        cb.error("failed to convert argument");
                        return;
                    };

                    if arg1.is_number() {
                        let Ok(arg1): Result<u64> = arg1.to_rust_value() else {
                            cb.error("failed to convert argument");
                            return;
                        };

                        let ret = s
                            .borrow()
                            .generic(arg0, arg1)
                            .to_js_value(ctx)
                            .unwrap();

                        cb.ret(ret);

                        return;
                    }

                    if arg1.is_string() {
                        let Ok(arg1): Result<String> = arg1.to_rust_value() else {
                            cb.error("failed to convert argument");
                            return;
                        };

                        let ret = s
                            .borrow()
                            .generic(arg0, arg1)
                            .to_js_value(ctx)
                            .unwrap();

                        cb.ret(ret);
                    }
                }
            })?
        };

        obj.set_method("generic", &generic)?;

        Ok(())
    }
}

#[test]
fn manual_js_interop() {
    let mut engine = V8Engine::new();
    let mut context = engine.new_context().unwrap();

    let t2 = Rc::new(RefCell::new(Test2 {
        field: 14,
        other_field: "Hello, ".to_string(),
    }));

    Test2::implement::<V8Engine>(t2.clone(), context.clone()).unwrap();

    let out = context
        .run(
            r#"
        test2.variadic(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
        test2.cool_fn() //  \
        test2.add(3)    //   |-> functions defined in rust
        test2.cool_fn() //  /
        test2.variadic(test2, test2.cool_fn, test2.cool_fn(), test2.field, test2.other_field)

        test2.field += 5
        test2.field = 33
        test2.field
        test2.other_field += "World!"
        test2.other_field
    "#,
        )
        .expect("no value")
        .as_string()
        .unwrap();

    println!("JS: {out}");
    println!("Rust: {:?}", t2.borrow());
}
