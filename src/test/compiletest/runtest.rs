import std::io;
import std::str;
import std::option;
import std::fs;
import std::os;
import std::vec;
import std::test;

import common::mode_run_pass;
import common::mode_run_fail;
import common::mode_compile_fail;
import common::mode_pretty;
import common::cx;
import common::config;
import header::load_props;
import header::test_props;
import util::logv;

export run;

fn run(cx: &cx, _testfile: -[u8]) {
    let testfile = str::unsafe_from_bytes(_testfile);
    if cx.config.verbose {
        // We're going to be dumping a lot of info. Start on a new line.
        io::stdout().write_str(~"\n\n");
    }
    log #ifmt["running %s", testfile];
    let props = load_props(testfile);
    alt cx.config.mode {
      mode_compile_fail. { run_cfail_test(cx, props, testfile); }
      mode_run_fail. { run_rfail_test(cx, props, testfile); }
      mode_run_pass. { run_rpass_test(cx, props, testfile); }
      mode_pretty. { run_pretty_test(cx, props, testfile); }
    }
}

fn run_cfail_test(cx: &cx, props: &test_props, testfile: &istr) {
    let procres = compile_test(cx, props, testfile);

    if procres.status == 0 {
        fatal_procres(~"compile-fail test compiled successfully!", procres);
    }

    check_error_patterns(props, testfile, procres);
}

fn run_rfail_test(cx: &cx, props: &test_props, testfile: &istr) {
    let procres = compile_test(cx, props, testfile);

    if procres.status != 0 { fatal_procres(~"compilation failed!", procres); }

    procres = exec_compiled_test(cx, props, testfile);

    if procres.status == 0 {
        fatal_procres(~"run-fail test didn't produce an error!", procres);
    }

    // This is the value valgrind returns on failure
    // FIXME: Why is this value neither the value we pass to
    // valgrind as --error-exitcode (1), nor the value we see as the
    // exit code on the command-line (137)?
    const valgrind_err: int = 9;
    if procres.status == valgrind_err {
        fatal_procres(~"run-fail test isn't valgrind-clean!", procres);
    }

    check_error_patterns(props, testfile, procres);
}

fn run_rpass_test(cx: &cx, props: &test_props, testfile: &istr) {
    let procres = compile_test(cx, props, testfile);

    if procres.status != 0 { fatal_procres(~"compilation failed!", procres); }

    procres = exec_compiled_test(cx, props, testfile);


    if procres.status != 0 { fatal_procres(~"test run failed!", procres); }
}

fn run_pretty_test(cx: &cx, props: &test_props, testfile: &istr) {
    if option::is_some(props.pp_exact) {
        logv(cx.config, ~"testing for exact pretty-printing");
    } else { logv(cx.config, ~"testing for converging pretty-printing"); }

    let rounds =
        alt props.pp_exact { option::some(_) { 1 } option::none. { 2 } };

    let srcs = [io::read_whole_file_str(testfile)];

    let round = 0;
    while round < rounds {
        logv(cx.config, #ifmt["pretty-printing round %d", round]);
        let procres = print_source(cx, testfile, srcs[round]);

        if procres.status != 0 {
            fatal_procres(
                    #ifmt["pretty-printing failed in round %d", round],
                    procres);
        }

        srcs += [procres.stdout];
        round += 1;
    }

    let expected =
        alt props.pp_exact {
          option::some(file) {
            let filepath = fs::connect(fs::dirname(testfile), file);
            io::read_whole_file_str(filepath)
          }
          option::none. { srcs[vec::len(srcs) - 2u] }
        };
    let actual = srcs[vec::len(srcs) - 1u];

    if option::is_some(props.pp_exact) {
        // Now we have to care about line endings
        let cr = ~"\r";
        check (str::is_not_empty(cr));
        actual = str::replace(actual, cr, ~"");
        expected = str::replace(expected, cr, ~"");
    }

    compare_source(expected, actual);

    // Finally, let's make sure it actually appears to remain valid code
    let procres = typecheck_source(cx, testfile, actual);

    if procres.status != 0 {
        fatal_procres(~"pretty-printed source does not typecheck", procres);
    }

    ret;

    fn print_source(cx: &cx, testfile: &istr, src: &istr) -> procres {
        compose_and_run(cx, testfile, make_pp_args,
                        cx.config.compile_lib_path, option::some(src))
    }

    fn make_pp_args(config: &config, _testfile: &istr) -> procargs {
        let prog = config.rustc_path;
        let args = [~"-", ~"--pretty", ~"normal"];
        ret {prog: prog, args: args};
    }

    fn compare_source(expected: &istr, actual: &istr) {
        if expected != actual {
            error(~"pretty-printed source does match expected source");
            let msg =
                #ifmt["\n\
expected:\n\
------------------------------------------\n\
%s\n\
------------------------------------------\n\
actual:\n\
------------------------------------------\n\
%s\n\
------------------------------------------\n\
\n",
                     expected,
                      actual];
            io::stdout().write_str(msg);
            fail;
        }
    }

    fn typecheck_source(cx: &cx, testfile: &istr, src: &istr) -> procres {
        compose_and_run(cx, testfile, make_typecheck_args,
                        cx.config.compile_lib_path, option::some(src))
    }

    fn make_typecheck_args(config: &config, _testfile: &istr) -> procargs {
        let prog = config.rustc_path;
        let args = [~"-", ~"--no-trans", ~"--lib"];
        ret {prog: prog, args: args};
    }
}

fn check_error_patterns(props: &test_props, testfile: &istr,
                        procres: &procres) {
    if vec::is_empty(props.error_patterns) {
        fatal(~"no error pattern specified in " + testfile);
    }

    if procres.status == 0 {
        fatal(~"process did not return an error status");
    }

    let next_err_idx = 0u;
    let next_err_pat = props.error_patterns[next_err_idx];
    for line: istr in str::split(procres.stdout, '\n' as u8) {
        if str::find(line, next_err_pat) > 0 {
            log #ifmt["found error pattern %s",
                      next_err_pat];
            next_err_idx += 1u;
            if next_err_idx == vec::len(props.error_patterns) {
                log "found all error patterns";
                ret;
            }
            next_err_pat = props.error_patterns[next_err_idx];
        }
    }

    let missing_patterns =
        vec::slice(props.error_patterns, next_err_idx,
                   vec::len(props.error_patterns));
    if vec::len(missing_patterns) == 1u {
        fatal_procres(
            #ifmt["error pattern '%s' not found!",
                  missing_patterns[0]], procres);
    } else {
        for pattern: istr in missing_patterns {
            error(#ifmt["error pattern '%s' not found!",
                        pattern]);
        }
        fatal_procres(~"multiple error patterns not found", procres);
    }
}

type procargs = {prog: istr, args: [istr]};

type procres = {status: int, stdout: istr, stderr: istr, cmdline: istr};

fn compile_test(cx: &cx, props: &test_props, testfile: &istr) -> procres {
    compose_and_run(cx, testfile, bind make_compile_args(_, props, _),
                    cx.config.compile_lib_path, option::none)
}

fn exec_compiled_test(cx: &cx, props: &test_props, testfile: &istr) ->
   procres {
    compose_and_run(cx, testfile, bind make_run_args(_, props, _),
                    cx.config.run_lib_path, option::none)
}

fn compose_and_run(cx: &cx, testfile: &istr,
                   make_args: fn(&config, &istr) -> procargs,
                   lib_path: &istr,
                   input: option::t<istr>) -> procres {
    let procargs = make_args(cx.config, testfile);
    ret program_output(cx, testfile, lib_path, procargs.prog, procargs.args,
                       input);
}

fn make_compile_args(config: &config, props: &test_props, testfile: &istr) ->
   procargs {
    let prog = config.rustc_path;
    let args = [testfile, ~"-o", make_exe_name(config, testfile)];
    let rustcflags = alt config.rustcflags {
      option::some(s) { option::some(s) }
      option::none. { option::none }
    };
    args += split_maybe_args(rustcflags);
    args += split_maybe_args(props.compile_flags);
    ret {prog: prog, args: args};
}

fn make_exe_name(config: &config, testfile: &istr) -> istr {
    output_base_name(config, testfile) + os::exec_suffix()
}

fn make_run_args(config: &config, props: &test_props, testfile: &istr) ->
   procargs {
    let toolargs = if !props.no_valgrind {
        // If we've got another tool to run under (valgrind),
        // then split apart its command
        let runtool = alt config.runtool {
          option::some(s) { option::some(s) }
          option::none. { option::none }
        };
        split_maybe_args(runtool)
    } else { [] };

    let args = toolargs + [make_exe_name(config, testfile)];
    ret {prog: args[0], args: vec::slice(args, 1u, vec::len(args))};
}

fn split_maybe_args(argstr: &option::t<istr>) -> [istr] {
    fn rm_whitespace(v: &[istr]) -> [istr] {
        fn flt(s: &istr) -> option::t<istr> {
            if !is_whitespace(s) { option::some(s) } else { option::none }
        }

        // FIXME: This should be in std
        fn is_whitespace(s: &istr) -> bool {
            for c: u8 in s { if c != ' ' as u8 { ret false; } }
            ret true;
        }
        vec::filter_map(flt, v)
    }

    alt argstr {
      option::some(s) { rm_whitespace(str::split(s, ' ' as u8)) }
      option::none. { [] }
    }
}

fn program_output(cx: &cx, testfile: &istr, lib_path: &istr, prog: &istr,
                  args: &[istr], input: option::t<istr>) -> procres {
    let cmdline =
        {
            let cmdline = make_cmdline(lib_path, prog, args);
            logv(cx.config, #ifmt["executing %s",
                                  cmdline]);
            cmdline
        };
    let res = procsrv::run(cx.procsrv, lib_path, prog, args, input);
    dump_output(cx.config, testfile, res.out, res.err);
    ret {status: res.status,
         stdout: res.out,
         stderr: res.err,
         cmdline: cmdline};
}

fn make_cmdline(libpath: &istr, prog: &istr, args: &[istr]) -> istr {
    #ifmt["%s %s %s",
          lib_path_cmd_prefix(libpath),
          prog,
          str::connect(args, ~" ")]
}

// Build the LD_LIBRARY_PATH variable as it would be seen on the command line
// for diagnostic purposes
fn lib_path_cmd_prefix(path: &istr) -> istr {
        #ifmt["%s=\"%s\"",
              util::lib_path_env_var(),
              util::make_new_path(path)]
}

fn dump_output(config: &config, testfile: &istr, out: &istr, err: &istr) {
    dump_output_file(config, testfile, out, ~"out");
    dump_output_file(config, testfile, err, ~"err");
    maybe_dump_to_stdout(config, out, err);
}

#[cfg(target_os = "win32")]
#[cfg(target_os = "linux")]
fn dump_output_file(config: &config, testfile: &istr, out: &istr,
                    extension: &istr) {
    let outfile = make_out_name(config, testfile, extension);
    let writer = io::file_writer(outfile,
                                 [io::create, io::truncate]);
    writer.write_str(out);
}

// FIXME (726): Can't use file_writer on mac
#[cfg(target_os = "macos")]
fn dump_output_file(config: &config, testfile: &istr, out: &istr,
                    extension: &istr) {
}

fn make_out_name(config: &config, testfile: &istr,
                 extension: &istr) -> istr {
    output_base_name(config, testfile) + ~"." + extension
}

fn output_base_name(config: &config, testfile: &istr) -> istr {
    let base = config.build_base;
    let filename =
        {
            let parts = str::split(fs::basename(testfile),
                                    '.' as u8);
            parts = vec::slice(parts, 0u, vec::len(parts) - 1u);
            str::connect(parts, ~".")
        };
    #ifmt["%s%s.%s", base, filename,
                        config.stage_id]
}

fn maybe_dump_to_stdout(config: &config, out: &istr, err: &istr) {
    if config.verbose {
        let sep1 = #ifmt["------%s------------------------------", ~"stdout"];
        let sep2 = #ifmt["------%s------------------------------", ~"stderr"];
        let sep3 = ~"------------------------------------------";
        io::stdout().write_line(sep1);
        io::stdout().write_line(out);
        io::stdout().write_line(sep2);
        io::stdout().write_line(err);
        io::stdout().write_line(sep3);
    }
}

fn error(err: &istr) {
    io::stdout().write_line(#ifmt["\nerror: %s", err]);
}

fn fatal(err: &istr) -> ! { error(err); fail; }

fn fatal_procres(err: &istr, procres: procres) -> ! {
    let msg =
        #ifmt["\n\
error: %s\n\
command: %s\n\
stdout:\n\
------------------------------------------\n\
%s\n\
------------------------------------------\n\
stderr:\n\
------------------------------------------\n\
%s\n\
------------------------------------------\n\
\n",
                             err,
                             procres.cmdline,
                             procres.stdout,
                             procres.stderr];
    io::stdout().write_str(msg);
    fail;
}
