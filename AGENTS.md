

* For function signatures that return anyhow::Result prefer fully qualified definition, for example this:
  ```rust
  fn my_function(i: u64) -> anyhow::Result<u64> {
      i + 2
  }
  ```
  and NOT THIS:
  ```rust
  use anyhow::Result
  
  fn my_function(i: u64) -> Result<u64> { // from line alone not clear is it some local result or any other type.
      i + 2
  }
  ```