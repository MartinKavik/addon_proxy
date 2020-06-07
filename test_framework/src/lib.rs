use proc_macro::TokenStream;
use syn::{parse_macro_input, parse_quote};
use quote::quote;
use std::mem;

/// Register setup callbacks.
///
/// # Example
///
// ```rust,ignore
/// use test_framework::test_callbacks;
///
/// #[test_callbacks]
/// #[cfg(test)]
/// mod integration {
///     // ------ SETUP ------
///    
///     fn before_all() {}
///
///     fn before_each() {}
///
///     fn after_each() {}
///
///     fn after_all() {)
///
///     // ------ TESTS ------
///
///     #[test]
///     fn it_works() {
///         assert_eq!(2 + 2, 4);
///     }
///
///     #[tokio::test]
///     async fn it_works_async() {
///         let two = futures::future::ready(2).await;
///         assert_eq!(two + 2, 4);
///     }
/// }
///```
///
/// # Generated code
///
// ```rust,ignore
/// use test_framework::test_callbacks;
///
/// #[test_callbacks]
/// #[cfg(test)]
/// mod integration {
///     // ------ SETUP ------
///    
///     fn before_all() {}
///
///     fn before_each() {}
///
///     fn after_each() {}
///
///     fn after_all() {)
///
///     // ------ TESTS ------
///
///     #[test]
///     fn it_works() {
///         run_test_sync(|| {assert_eq!(2 + 2, 4);});
///     }
///
///     #[tokio::test]
///     fn it_works_async() {
///         run_test_async(async {
///             let two = futures::future::ready(2).await;
///             assert_eq!(two + 2, 4);
///         }).await;
///     }
///     const TEST_COUNT: usize = 1;
///     static REMAINING_TESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(TEST_COUNT);
///     static BEFORE_ALL_CALL: std::sync::Once = std::sync::Once::new();
///     fn run_test_sync(test: impl FnOnce() + std::panic::UnwindSafe) {
///         BEFORE_ALL_CALL.call_once(|| {
///             before_all();
///         });
///         before_each();  
///
///         let result = std::panic::catch_unwind(|| {
///             test()
///         });    
///
///         after_each();   
///         if REMAINING_TESTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 1 {
///             after_all();
///         }
///         
///         assert!(result.is_ok())
///     }    
///     async fn run_test_async(test: impl futures::future::Future)
///     {
///         BEFORE_ALL_CALL.call_once(|| {
///             before_all();
///         });
///         before_each();  
///
///         use futures::future::FutureExt;
///         let result = std::panic::AssertUnwindSafe(test).catch_unwind().await; 
///
///         after_each();   
///         if REMAINING_TESTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 1 {
///             after_all();
///         }
///            
///         assert!(result.is_ok())
///     }    
/// }
///```
#[proc_macro_attribute]
pub fn test_callbacks(_: TokenStream, tokens: TokenStream) -> TokenStream {
    let mut item_mod = parse_macro_input!(tokens as syn::ItemMod);

    if let Some((_, items)) = &mut item_mod.content {
        let mut test_count: usize = 0;

        for item in items.iter_mut() {
            if let syn::Item::Fn(item_fn) = item {
                // Functions with an attribute that contains "test" are considered as tests.
                let is_test = item_fn.attrs.iter().any(|attr| {
                    attr.path.segments.iter().any(|segment| {
                        segment.ident == "test"
                    })
                });
                if is_test {
                    test_count += 1;

                    // Wrap original test block into async or sync wrapper.

                    let stmts = mem::take(&mut item_fn.block.stmts);
                    let nested_block = syn::Block {
                        brace_token: item_fn.block.brace_token,
                        stmts
                    };
                    let wrapped_stmts = if item_fn.sig.asyncness.is_some() {
                        parse_quote! {
                            run_test_async(async #nested_block).await;
                        }
                    } else {
                        parse_quote! {
                            run_test_sync(|| #nested_block);
                        }
                    };
                    item_fn.block.stmts = vec![wrapped_stmts];
                }
            }
        }

        // ------ Inject `TEST_COUNT`, `REMAINING_TESTS` and `BEFORE_ALL_CALL` ------

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

        // ------ Inject `run_test_sync` and `run_test_async` ------

        let item_fn_run_test_sync = parse_quote! {
            fn run_test_sync(test: impl FnOnce() + std::panic::UnwindSafe) {
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
        items.push(syn::Item::Fn(item_fn_run_test_sync));

        let item_fn_run_test_async = parse_quote! {
            async fn run_test_async(test: impl futures::future::Future)
            {
                BEFORE_ALL_CALL.call_once(|| {
                    before_all();
                });
                before_each();  
        
                use futures::future::FutureExt;
                let result = std::panic::AssertUnwindSafe(test).catch_unwind().await; 
        
                after_each();   
                if REMAINING_TESTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 1 {
                    after_all();
                }
                
                assert!(result.is_ok())
            }    
        };    
        items.push(syn::Item::Fn(item_fn_run_test_async));
    }

    let output = quote!{ #item_mod };
    output.into()
}
