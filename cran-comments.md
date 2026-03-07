## R CMD check results

0 errors | 0 notes (local); see below for known check warnings.

## Known warnings

### checking compiled code

`abort`, `exit`, `_exit`, `stderr` originate from the tokio and object_store
Rust crates (async runtime and cloud I/O), which are required dependencies.
These are not called from package code directly and do not terminate R.

Non-API R calls (`BODY`, `CLOENV`, `DATAPTR`, `ENCLOS`, `FORMALS`) originate
from extendr-api 0.7.1, the R/Rust interop layer. These are addressed in
extendr >= 0.8 and will be resolved on the next extendr version bump.

### checking Rust compilation

`rustc --version --verbose` is now emitted during compilation via Makevars.

## Downstream dependencies

None.
