// We need to forward routine registration from the package to the library.
// Packages that use extendr need to call `R_init_rustycogs` here.

void R_init_rustycogs_extendr(void *dll);

void R_init_rustycogs(void *dll) {
    R_init_rustycogs_extendr(dll);
}
