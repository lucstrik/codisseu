use super::*;
use std::{fs, thread, time::Duration};

fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn parse(xml: &str, root: &Path) -> Vec<TestCase> {
    report::parse(xml.as_bytes(), root).unwrap()
}
fn suite(body: &str) -> String {
    format!("<testsuites><testsuite name=\"own\">{body}</testsuite></testsuites>")
}

#[test]
fn junit_preserves_unicode_entities_outcomes_and_real_failure_location() {
    let dir = root();
    fs::write(dir.path().join("ação Test.php"), "<?php\n// própria\n").unwrap();
    let xml = suite(
        r#"<testcase name="sum &amp; ação 😀" classname="Own" file="ação Test.php" line="1" time="0.125"/>
        <testcase name="failure" class="Own" file="ação Test.php" line="1"><failure type="Assertion" message="expected &lt; actual"><![CDATA[at ação Test.php:2]]></failure></testcase>
        <testcase name="error"><error type="RuntimeException">erro &#x1F600;</error></testcase>
        <testcase name="skip"><skipped message="reason"/></testcase>"#,
    );
    let cases = parse(&xml, dir.path());
    assert_eq!(cases.len(), 4);
    assert_eq!(cases[0].name, "sum & ação 😀");
    assert_eq!(cases[0].class, "Own");
    assert_eq!(cases[0].seconds, 0.125);
    assert_eq!(cases[0].outcome, Outcome::Passed);
    assert_eq!(cases[1].outcome, Outcome::Failed);
    assert_eq!(cases[1].line, Some(2));
    assert!(cases[1].details.contains("expected < actual"));
    assert_eq!(cases[2].outcome, Outcome::Error);
    assert!(cases[2].details.contains("erro 😀"));
    assert_eq!(cases[3].outcome, Outcome::Skipped);
    assert!(cases[3].details.contains("reason"));
}

#[test]
fn junit_pest_file_suffix_is_metadata_and_suite_file_is_inherited() {
    let dir = root();
    let path = dir.path().join("ExampleTest.php");
    fs::write(&path, "<?php\n").unwrap();
    let xml = r#"<testsuite file="ExampleTest.php"><testcase name="natural description" file="ExampleTest.php::natural description"/><testcase name="inherited"/></testsuite>"#;
    let cases = parse(xml, dir.path());
    assert_eq!(cases[0].path.as_ref(), Some(&path));
    assert_eq!(cases[1].path.as_ref(), Some(&path));
    assert_eq!(
        cases[0].line, None,
        "absence of a source line must remain unknown"
    );
}

#[test]
fn junit_rejects_malformed_unsafe_and_incomplete_xml() {
    let dir = root();
    for xml in [
        "",
        "<unexpected/>",
        "<testsuite>",
        "<testsuite></testsuites>",
        "<testsuite/><testsuite/>",
        "<testsuite><testcase/></testsuite>",
        "<testsuites><testcase name='outside'/></testsuites>",
        "<testsuite><testcase name='a'><testcase name='nested'/></testcase></testsuite>",
        "<testsuite><testcase name='a' name='b'/></testsuite>",
        "<!DOCTYPE testsuite [<!ENTITY xxe SYSTEM 'file:///etc/passwd'>]><testsuite/>",
        "<testsuite><testcase name='a'><failure>&unknown;</failure></testcase></testsuite>",
    ] {
        assert!(
            report::parse(xml.as_bytes(), dir.path()).is_err(),
            "accepted {xml}"
        );
    }
    assert!(report::parse(b"\xff", dir.path()).is_err());
}

#[test]
fn junit_bounds_cases_depth_attributes_and_details() {
    let dir = root();
    let deep = format!("{}{}", "<testsuite>".repeat(65), "</testsuite>".repeat(65));
    assert!(report::parse(deep.as_bytes(), dir.path()).is_err());
    let many = suite(&"<testcase name='a'/>".repeat(MAX_CASES + 1));
    assert!(report::parse(many.as_bytes(), dir.path()).is_err());
    let huge_name = suite(&format!("<testcase name='{}'/>", "x".repeat(4097)));
    assert!(report::parse(huge_name.as_bytes(), dir.path()).is_err());
    let detail = suite(&format!(
        "<testcase name='a'><failure>{}</failure></testcase>",
        "ação😀".repeat(20_000)
    ));
    let cases = parse(&detail, dir.path());
    assert!(cases[0].details.len() <= 64 * 1024);
    assert!(!cases[0].details.is_empty());
    assert_eq!(cases[0].outcome, Outcome::Failed);
}

#[test]
fn junit_never_navigates_to_external_or_symlinked_source() {
    let dir = root();
    let outside = root();
    let file = outside.path().join("External.php");
    fs::write(&file, "<?php").unwrap();
    let xml = suite(&format!(
        "<testcase name='external' file='{}' line='12'><failure>at {}:12</failure></testcase>",
        file.display(),
        file.display()
    ));
    assert!(parse(&xml, dir.path())[0].path.is_none());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&file, dir.path().join("Linked.php")).unwrap();
        assert!(
            parse(
                &suite("<testcase name='linked' file='Linked.php'/>"),
                dir.path()
            )[0]
            .path
            .is_none()
        );
    }
}

#[test]
fn junit_bad_timing_and_lines_do_not_invent_coordinates() {
    let dir = root();
    fs::write(dir.path().join("File.php"), "<?php").unwrap();
    for time in ["NaN", "inf", "-1", "not-a-number"] {
        let xml = suite(&format!(
            "<testcase name='a' time='{time}' file='File.php' line='0'/>"
        ));
        let case = parse(&xml, dir.path()).remove(0);
        assert_eq!(case.seconds, 0.);
        assert_eq!(case.line, None);
    }
}

#[test]
fn inspect_is_filesystem_only_and_rejects_foreign_root_without_execution() {
    let dir = root();
    let outside = root();
    fs::create_dir_all(dir.path().join("vendor/bin")).unwrap();
    fs::write(
        dir.path().join("vendor/bin/pest"),
        "<?php file_put_contents('unexpected', 'run');",
    )
    .unwrap();
    let mut service = Service::new(dir.path().join("profile/settings.json"));
    service
        .dispatch(
            TestCommand::Inspect(dir.path().into()),
            vec![dir.path().into()],
        )
        .unwrap();
    wait_service(&mut service);
    assert_eq!(service.state, State::Idle);
    assert_eq!(service.config.as_ref().unwrap().root, dir.path());
    assert!(!dir.path().join("unexpected").exists());
    service
        .dispatch(
            TestCommand::Inspect(outside.path().into()),
            vec![dir.path().into()],
        )
        .unwrap();
    wait_service(&mut service);
    assert_eq!(service.state, State::Error);
    assert!(service.report.error.as_deref().unwrap().contains("fora"));
}

fn wait_service(service: &mut Service) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "PHP service timeout");
        thread::sleep(Duration::from_millis(3));
    }
}

#[cfg(unix)]
fn controlled_config(dir: &Path, script: &str) -> Config {
    fs::write(dir.join("runner.py"), script).unwrap();
    let mut config = Config::new(dir.into());
    config.php = "/usr/bin/python3".into();
    config.runner = "runner.py".into();
    config
}
#[cfg(unix)]
fn run_controlled(dir: &Path, config: Config) -> Service {
    let mut service = Service::new(dir.join("profile/settings.json"));
    service
        .dispatch(
            TestCommand::Run {
                config,
                scope: Scope::Suite,
            },
            vec![dir.into()],
        )
        .unwrap();
    wait_service(&mut service);
    service
}

#[cfg(unix)]
#[test]
fn controlled_runner_exit_code_missing_report_zero_cases_and_persisted_config() {
    let dir = root();
    let config = controlled_config(
        dir.path(),
        "import sys\nprint('not a report')\nsys.exit(7)\n",
    );
    let mut service = run_controlled(dir.path(), config.clone());
    assert_eq!(service.state, State::Error);
    assert_eq!(service.report.exit_codes, vec![Some(7)]);
    assert!(service.report.error.as_deref().unwrap().contains("JUnit"));
    assert!(service.report.cases.is_empty());
    service = Service::new(dir.path().join("profile/settings.json"));
    service
        .dispatch(
            TestCommand::Inspect(dir.path().into()),
            vec![dir.path().into()],
        )
        .unwrap();
    wait_service(&mut service);
    assert_eq!(service.config, Some(config));
    for (body, exit) in [
        ("<testsuites/>", 0),
        ("<testsuite><testcase name='pass'/></testsuite>", 7),
    ] {
        let script = format!(
            "import sys,pathlib\np=sys.argv[sys.argv.index('--log-junit')+1]\npathlib.Path(p).write_text({body:?})\nsys.exit({exit})\n"
        );
        let config = controlled_config(dir.path(), &script);
        let result = run_controlled(dir.path(), config);
        assert_eq!(result.state, State::Error);
        assert!(result.report.error.is_some());
        assert_eq!(result.report.exit_codes, vec![Some(exit)]);
    }
}

#[cfg(unix)]
#[test]
fn controlled_runner_bad_configuration_does_not_start_and_argv_is_literal() {
    let dir = root();
    let config = controlled_config(
        dir.path(),
        "import pathlib\npathlib.Path('STARTED').write_text('yes')\n",
    );
    let mut invalid = config.clone();
    invalid.timeout_secs = 0;
    let result = run_controlled(dir.path(), invalid);
    assert_eq!(result.state, State::Error);
    assert!(!dir.path().join("STARTED").exists());
    let mut invalid = config.clone();
    invalid.xml = "missing.xml".into();
    let result = run_controlled(dir.path(), invalid);
    assert_eq!(result.state, State::Error);
    assert!(!dir.path().join("STARTED").exists());
    let odd = dir
        .path()
        .join("literal $(touch SHOULD_NOT_EXIST); 'ação'.py");
    fs::write(&odd, "import sys,pathlib,json\npathlib.Path('argv.json').write_text(json.dumps(sys.argv))\np=sys.argv[sys.argv.index('--log-junit')+1]\npathlib.Path(p).write_text(\"<testsuite><testcase name='ok'/></testsuite>\")\n").unwrap();
    let mut literal = config;
    literal.runner = odd.file_name().unwrap().to_str().unwrap().into();
    let result = run_controlled(dir.path(), literal);
    assert_eq!(result.state, State::Passed, "{:?}", result.report.error);
    let args: Vec<String> =
        serde_json::from_str(&fs::read_to_string(dir.path().join("argv.json")).unwrap()).unwrap();
    assert_eq!(args[0], odd.to_str().unwrap());
    assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
}

#[cfg(unix)]
#[test]
fn controlled_process_output_budget_and_ansi_are_inert() {
    let dir = root();
    let (sender, _receiver) = mpsc::sync_channel(32);
    let code = "import os\nos.write(1,b'\\x1b[31mred\\x1b[0m\\x1b]52;c;secret\\x07\\n')\nos.write(1,b'x'*1200000)\nos.write(2,b'err')";
    let result = process::run(
        Path::new("/usr/bin/python3"),
        &["-c".into(), code.into()],
        dir.path(),
        Duration::from_secs(5),
        &AtomicBool::new(false),
        &sender,
    )
    .unwrap();
    assert!(result.success);
    assert!(result.truncated);
    assert!(result.text.len() <= MAX_OUTPUT);
    assert!(result.text.starts_with("red\n"));
    assert!(!result.text.contains(['\u{1b}', '\u{7}']));
    assert!(!result.text.contains("secret"));
}

#[cfg(target_os = "linux")]
#[test]
fn controlled_process_timeout_and_cancel_reap_owned_group() {
    for cancel_early in [false, true] {
        let dir = root();
        let marker = dir.path().join("descendant.pid");
        let code = "import subprocess,pathlib,time\np=subprocess.Popen(['/bin/sleep','30'])\npathlib.Path('descendant.pid').write_text(str(p.pid))\nprint('ready',flush=True)\ntime.sleep(30)";
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        let watch = marker.clone();
        let watcher = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !watch.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if cancel_early {
                thread_cancel.store(true, Ordering::SeqCst);
            }
        });
        let (sender, _receiver) = mpsc::sync_channel(32);
        let began = Instant::now();
        let output = process::run(
            Path::new("/usr/bin/python3"),
            &["-c".into(), code.into()],
            dir.path(),
            Duration::from_millis(250),
            &cancel,
            &sender,
        )
        .unwrap();
        watcher.join().unwrap();
        assert!(began.elapsed() < Duration::from_secs(3));
        assert!(!output.success);
        assert_eq!(output.cancelled, cancel_early);
        assert_eq!(output.timeout, !cancel_early);
        let pid: u32 = fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
        // SIGKILL delivery to another member of the process group is
        // asynchronous even after its parent has been waited. Keep the same
        // termination assertion, allowing the kernel to schedule that member.
        let deadline = Instant::now() + Duration::from_secs(1);
        while let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
            if stat.split_whitespace().nth(2) == Some("Z") {
                break;
            }
            assert!(Instant::now() < deadline, "descendant survived: {stat}");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

#[test]
fn junit_conflicting_outcomes_cannot_hide_a_failure() {
    let dir = root();
    for outcomes in [
        "<failure/><skipped/>",
        "<error/><failure/>",
        "<skipped/><skipped/>",
    ] {
        let xml = suite(&format!("<testcase name='own'>{outcomes}</testcase>"));
        assert!(
            report::parse(xml.as_bytes(), dir.path()).is_err(),
            "accepted conflicting outcomes {outcomes}"
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires real PHP PCRE and installed fixture opt-in"]
fn real_php_pcre_filters_are_literal_exact_and_class_scoped() {
    assert!(
        std::env::var_os("CODISSEU_PHP_TEST_FIXTURE").is_some(),
        "set CODISSEU_PHP_TEST_FIXTURE"
    );
    let php = std::env::var("CODISSEU_PHP_TEST_PHP").unwrap_or_else(|_| "/usr/bin/php".into());
    let name = "with [brackets] (a+b) ~ $. ação 😀";
    let filter = literal_filter(
        &[("Example\\First".into(), name.into())],
        Framework::PhpUnit,
    )
    .unwrap();
    let probes = vec![
        format!("Example\\First::{name}"),
        format!("Example\\Second::{name}"),
        format!("prefixExample\\First::{name}"),
        format!("Example\\First::{name} suffix"),
    ];
    let output = std::process::Command::new(&php).args(["-r", "$p=$argv[1];$v=json_decode($argv[2],true,512,JSON_THROW_ON_ERROR);echo json_encode(array_map(fn($s)=>preg_match($p,$s),$v));", &filter, &serde_json::to_string(&probes).unwrap()]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Vec<i32>>(&output.stdout).unwrap(),
        vec![1, 0, 0, 0]
    );
    let filter = literal_filter(&[("JUnit\\Class".into(), name.into())], Framework::Pest).unwrap();
    let probes = vec![
        format!("P\\GeneratedClass::{name}"),
        format!("P\\GeneratedClass::{name} extra"),
    ];
    let output = std::process::Command::new(&php).args(["-r", "$p=$argv[1];$v=json_decode($argv[2],true,512,JSON_THROW_ON_ERROR);echo json_encode(array_map(fn($s)=>preg_match($p,$s),$v));", &filter, &serde_json::to_string(&probes).unwrap()]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Vec<i32>>(&output.stdout).unwrap(),
        vec![1, 0]
    );
}

#[cfg(unix)]
#[test]
fn junit_preserves_real_paths_containing_double_colon() {
    let dir = root();
    let folder = dir.path().join("project::ação");
    fs::create_dir(&folder).unwrap();
    let path = folder.join("Example::Test.php");
    fs::write(&path, "<?php\n").unwrap();
    let normal = suite(&format!(
        "<testcase name='normal' file='{}' line='1'/>",
        path.display()
    ));
    assert_eq!(parse(&normal, dir.path())[0].path.as_ref(), Some(&path));
    let pest = suite(&format!(
        "<testcase name='natural::description' file='{}::natural::description'/>",
        path.display()
    ));
    assert_eq!(parse(&pest, dir.path())[0].path.as_ref(), Some(&path));
}

#[test]
fn junit_rejects_non_whitespace_outside_document_root() {
    let dir = root();
    for xml in [
        "junk<testsuite><testcase name='pass'/></testsuite>",
        "<testsuite><testcase name='pass'/></testsuite>junk",
        "<testsuite><testcase name='pass'/></testsuite><![CDATA[junk]]>",
    ] {
        assert!(
            report::parse(xml.as_bytes(), dir.path()).is_err(),
            "accepted invalid outer text: {xml}"
        );
    }
    assert_eq!(
        parse(
            "\n<testsuite><testcase name='pass'/></testsuite>\n",
            dir.path()
        )
        .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn inspect_new_project_clears_previous_report_and_rerun_identity() {
    let first = root();
    let second = root();
    let config = controlled_config(
        first.path(),
        "import pathlib,sys\np=sys.argv[sys.argv.index('--log-junit')+1]\npathlib.Path(p).write_text(\"<testsuite><testcase name='first project'/></testsuite>\")\n",
    );
    let mut service = run_controlled(first.path(), config);
    assert_eq!(service.report.cases.len(), 1);
    service
        .dispatch(
            TestCommand::Inspect(second.path().into()),
            vec![first.path().into(), second.path().into()],
        )
        .unwrap();
    wait_service(&mut service);
    assert_eq!(service.config.as_ref().unwrap().root, second.path());
    assert!(service.report.cases.is_empty());
    assert!(service.report.output.is_empty());
    assert!(service.command.is_empty());
    assert!(service.last_root().is_none());
    assert!(
        service
            .dispatch(TestCommand::RerunLast, vec![second.path().into()])
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn auto_rerun_preserves_resolved_framework_if_project_gains_pest() {
    let dir = root();
    fs::create_dir_all(dir.path().join("vendor/bin")).unwrap();
    fs::write(dir.path().join("Source.php"), "<?php\n").unwrap();
    fs::write(dir.path().join("vendor/bin/phpunit"), "import pathlib,sys\np=sys.argv[sys.argv.index('--log-junit')+1]\npathlib.Path(p).write_text(\"<testsuite><testcase name='testSame' class='Own' file='Source.php'/></testsuite>\")\n").unwrap();
    let mut config = Config::new(dir.path().into());
    config.php = "/usr/bin/python3".into();
    let mut service = run_controlled(dir.path(), config);
    assert_eq!(service.state, State::Passed);
    assert_eq!(
        service.config.as_ref().unwrap().framework,
        Framework::PhpUnit
    );
    fs::write(
        dir.path().join("vendor/bin/pest"),
        "import pathlib\npathlib.Path('WRONG_RUNNER').write_text('unexpected')\n",
    )
    .unwrap();
    service
        .dispatch(TestCommand::RunSelected(0), vec![dir.path().into()])
        .unwrap();
    wait_service(&mut service);
    assert_eq!(service.state, State::Passed, "{:?}", service.report.error);
    assert!(!dir.path().join("WRONG_RUNNER").exists());
}

#[test]
fn disconnected_worker_cannot_leave_service_running_forever() {
    let dir = root();
    let mut service = Service::new(dir.path().join("profile.json"));
    let (sender, receiver) = mpsc::sync_channel::<Event>(1);
    drop(sender);
    service.receiver = Some(receiver);
    service.state = State::Running;
    assert!(service.poll());
    assert!(!service.is_busy());
    assert_eq!(service.state, State::Error);
    assert!(service.report.error.is_some());
}
