use proc_macro::TokenStream;
use syn::{parse_macro_input, parse_quote};
use quote::quote;
use std::mem;

/// Register setup callbacks.
///
/// # Example
///
/// use test_framework::tests;
///
/// tests!{
///     #[cfg(test)]
///     mod integration {
///         // ------ SETUP ------
///        
///         fn before_all() {}
///
///         fn before_each() {}
///
///         fn after_each() {}
///
///         fn after_all() {)
///
///         // ------ TESTS ------
///
///         #[test]
///         fn it_works() {
///             assert_eq!(2 + 2, 4);
///         }
///     }
/// }
#[proc_macro_attribute]
pub fn test_callbacks(_: TokenStream, tokens: TokenStream) -> TokenStream {
    let mut item_mod = parse_macro_input!(tokens as syn::ItemMod);

    if let Some((_, items)) = &mut item_mod.content {
        let mut test_count: usize = 0;

        for item in items.iter_mut() {
            if let syn::Item::Fn(item_fn) = item {
                if item_fn.attrs.iter().any(|attr| attr.path.is_ident("test")) {
                    test_count += 1;

                    let stmts = mem::take(&mut item_fn.block.stmts);
                    let nested_block = syn::Block {
                        brace_token: item_fn.block.brace_token,
                        stmts
                    };
                    let wrapped_stmts = parse_quote! {
                        run_test_(|| #nested_block);
                    };
                    item_fn.block.stmts = vec![wrapped_stmts];
                }
            }
        }

        let item_test_count = parse_quote! {
            const TEST_COUNT: usize = #test_count;
        };
        items.push(syn::Item::Const(item_test_count));

        let item_remaining_tests = parse_quote! {
            static REMAINING_TESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(TEST_COUNT);
        };
        items.push(syn::Item::Static(item_remaining_tests));

        let item_before_all_call = parse_quote! {
            static BEFORE_ALL_CALL: std::sync::Once = std::sync::Once::new();
        };
        items.push(syn::Item::Static(item_before_all_call));

        let item_fn_run_test_ = parse_quote! {
            fn run_test_<T>(test: T) -> ()
                where T: FnOnce() -> () + std::panic::UnwindSafe
            {
                BEFORE_ALL_CALL.call_once(|| {
                    before_all();
                });
                before_each();  

                let result = std::panic::catch_unwind(|| {
                    test()
                });    

                after_each();   
                if REMAINING_TESTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 1 {
                    after_all();
                }
                
                assert!(result.is_ok())
            }    
        };    
        items.push(syn::Item::Fn(item_fn_run_test_));
    }

    let output = quote!{ #item_mod };
    output.into()
}
