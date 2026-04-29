
* Prefer idiomatic rust code (Refer)
* Utilize type system to eliminate classes of bugs at compile time

* NEVER type alias `Result`. 
  * good:
    ```rust
    fn foo() -> Result<(), MyError> {}
    fn bar() -> anyhow::Result<()> {}
    ```
  * bad
    ```rust
    type Result<T> = std::result::Result<T, MyError>;

    // 100 lines below
    fn foo() -> Result<()> { // Is it anyhow::Result, or crate local result, or what? SAVING IN 
    }
    ```
  * Same goes for anyhow: For function signatures that return anyhow::Result prefer fully qualified definition, for example this:
    ```rust
    fn my_function(i: u64) -> anyhow::Result<u64> {
        i + 2
    }
    ```
    and NOT THIS:
    ```rust
    use anyhow::Result;
  
    fn my_function(i: u64) -> Result<u64> { // from line alone not clear is it some local result or any other type.
        i + 2
    }
    ```
  
* For errors always prefer to put what you "got", not only you've expected. This speeds up future debugging. 
  Bad input is already in the error message, this saves a cycle of running it again with debugging. For example:
  * bad:
    ```rust
    
    ```
  * good