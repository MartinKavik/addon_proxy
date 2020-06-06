use test_framework::test_callbacks;

#[test_callbacks]
#[cfg(test)]
mod integration {
    // ------ SETUP ------
    
    fn before_all() {
        eprintln!("BEFORE ALL!");
    }

    fn before_each() {
        eprintln!("BEFORE EACH!");
    }

    fn after_each() {
        eprintln!("AFTER EACH!");
    }

    fn after_all() {
        eprintln!("AFTER ALL!");
    }

    // ------ TESTS ------

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn it_works_2() {
        assert_eq!(2 + 2, 4);
    }
}
