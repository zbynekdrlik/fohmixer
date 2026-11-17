use std::sync::Mutex;

use super::*;

fn target(instance: &str, track: &str, listeners: usize) -> Target {
    Target {
        instance: instance.into(),
        set: "FOH Band".into(),
        track: track.into(),
        listeners,
    }
}

fn sw(instance: &str, track: &str, on: bool) -> Switch {
    Switch {
        instance: instance.into(),
        track: track.into(),
        on,
    }
}

fn item(class: &str, name: &str, path: &str) -> Value {
    json!({"$ref": "r1", "class": class, "name": name, "path": path})
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

#[test]
fn a_new_target_switches_its_rack_on_and_a_join_nothing() {
    let vox = target("band", "Vox 1", 1);
    assert_eq!(
        start_switches(Start::New, None, &vox),
        vec![sw("band", "Vox 1", true)]
    );
    assert!(start_switches(Start::Join, Some(&vox), &vox).is_empty());
}

#[test]
fn a_take_switches_the_new_rack_on_first_then_the_old_one_off() {
    let vox = target("band", "Vox 1", 2);
    let keys = target("band", "Keys", 1);
    assert_eq!(
        start_switches(Start::Take, Some(&vox), &keys),
        vec![sw("band", "Keys", true), sw("band", "Vox 1", false)]
    );
    // The same track name in the other Live is another rack.
    let master = target("master", "Vox 1", 1);
    assert_eq!(
        start_switches(Start::Take, Some(&vox), &master),
        vec![sw("master", "Vox 1", true), sw("band", "Vox 1", false)]
    );
    // The same track of the same Live (its set loaded anew): never off.
    let reloaded = Target {
        instance: "band".into(),
        set: "Other set".into(),
        track: "Vox 1".into(),
        listeners: 1,
    };
    assert_eq!(
        start_switches(Start::Take, Some(&vox), &reloaded),
        vec![sw("band", "Vox 1", true)]
    );
}

#[test]
fn the_rack_goes_off_only_when_the_last_listener_left() {
    assert_eq!(
        leave_switch(&target("band", "Vox 1", 0)),
        Some(sw("band", "Vox 1", false))
    );
    assert_eq!(leave_switch(&target("band", "Vox 1", 1)), None);
}

#[test]
fn the_tracks_returns_and_main_are_read_by_the_name_escaped() {
    let read = |target: &str, prop: &str| json!({"target": target, "name": "get_prop", "args": {"prop": prop}});
    assert_eq!(
        read_commands("Vox ]1"),
        vec![
            read("live_set tracks[name=Vox \\]1]", "devices"),
            read("live_set return_tracks[name=Vox \\]1]", "devices"),
            read("live_set master_track", "name"),
            read("live_set master_track", "devices"),
        ]
    );
}

fn not_found(list: &str) -> Value {
    json!({"ok": false, "error": format!("not found: {list}[name=Vox 1]")})
}

/// The read's answer: the tracks', the returns', the main's name and its
/// devices.
fn answer(tracks: Value, returns: Value, main: &str, main_devices: Value) -> Vec<Value> {
    vec![tracks, returns, ok(json!(main)), main_devices]
}

fn rack_at(path: &str) -> Value {
    ok(json!([item("RackDevice", "Listen", path)]))
}

#[test]
fn the_one_rack_device_named_listen_is_the_rack() {
    let devices = ok(json!([
        item("PluginDevice", "Listen", "live_set tracks 1 devices 0"),
        item("RackDevice", "Vocal FX", "live_set tracks 1 devices 1"),
        item("RackDevice", "Listen", "live_set tracks 1 devices 2"),
        item("RackDevice", "listen", "live_set tracks 1 devices 3"),
    ]));
    let main = rack_at("live_set master_track devices 0");
    let slots = answer(devices, not_found("return_tracks"), "Main", main);
    assert_eq!(
        rack_of("Vox 1", &slots),
        Ok("live_set tracks 1 devices 2".into()),
        "the main's rack is not Vox 1's"
    );
}

#[test]
fn a_returns_or_the_mains_rack_is_found_by_its_name() {
    let slots = answer(
        not_found("tracks"),
        rack_at("live_set return_tracks 0 devices 1"),
        "Main",
        ok(json!([])),
    );
    assert_eq!(
        rack_of("Vox 1", &slots),
        Ok("live_set return_tracks 0 devices 1".into())
    );
    let slots = answer(
        not_found("tracks"),
        not_found("return_tracks"),
        "Vox 1",
        rack_at("live_set master_track devices 0"),
    );
    assert_eq!(
        rack_of("Vox 1", &slots),
        Ok("live_set master_track devices 0".into())
    );
}

#[test]
fn no_rack_or_two_or_an_ambiguous_name_is_missing() {
    let none = ok(json!([item(
        "PluginDevice",
        "Pro-Q 4",
        "live_set tracks 1 devices 0"
    )]));
    let slots = answer(none, not_found("return_tracks"), "Main", ok(json!([])));
    assert_eq!(
        rack_of("Vox 1", &slots),
        Err(Outcome::Missing(NO_RACK.into()))
    );
    let nowhere = answer(
        not_found("tracks"),
        not_found("return_tracks"),
        "Main",
        rack_at("live_set master_track devices 0"),
    );
    assert_eq!(
        rack_of("Vox 1", &nowhere),
        Err(Outcome::Missing(NO_RACK.into()))
    );
    let two = answer(
        rack_at("live_set tracks 1 devices 0"),
        rack_at("live_set return_tracks 0 devices 0"),
        "Main",
        ok(json!([])),
    );
    assert_eq!(
        rack_of("Vox 1", &two),
        Err(Outcome::Missing(TWO_RACKS.into()))
    );
    let ambiguous = json!({"ok": false, "error": "ambiguous: tracks[name=Vox 1]"});
    let slots = answer(ambiguous, not_found("return_tracks"), "Main", ok(json!([])));
    assert_eq!(
        rack_of("Vox 1", &slots),
        Err(Outcome::Missing("ambiguous: tracks[name=Vox 1]".into()))
    );
    assert_eq!(
        rack_of("Vox 1", &[]),
        Err(Outcome::Missing("no result".into()))
    );
    assert_eq!(
        (NO_RACK, TWO_RACKS, RACK_NAME),
        ("no Listen rack", "two Listen racks", "Listen")
    );
}

#[test]
fn device_on_is_parameter_0s_value() {
    assert_eq!(
        write_commands("live_set tracks 1 devices 2", true),
        vec![
            json!({"target": "live_set tracks 1 devices 2 parameters 0", "name": "set_prop", "args": {"prop": "value", "value": 1.0}})
        ]
    );
    assert_eq!(
        write_commands("live_set tracks 1 devices 2", false)[0]["args"]["value"],
        0.0
    );
}

#[test]
fn a_refused_write_failed() {
    assert_eq!(written(&[json!({"ok": true, "data": null})]), Outcome::Done);
    assert_eq!(
        written(&[json!({"ok": false, "error": "Value cannot be set"})]),
        Outcome::Failed("Value cannot be set".into())
    );
    assert_eq!(written(&[]), Outcome::Failed("no result".into()));
}

#[test]
fn the_record_names_the_switch_and_its_outcome() {
    let on = sw("band", "Vox 1", true);
    assert_eq!(
        rack_fields(&on, &Outcome::Done),
        json!({"what": "rack", "instance": "band", "track": "Vox 1", "on": true, "result": "done", "problem": null})
    );
    let missing = rack_fields(&on, &Outcome::Missing(NO_RACK.into()));
    assert_eq!(
        (missing["result"].clone(), missing["problem"].clone()),
        (json!("missing"), json!(NO_RACK))
    );
    let failed = rack_fields(
        &sw("band", "Vox 1", false),
        &Outcome::Failed("offline".into()),
    );
    assert_eq!(
        (
            failed["on"].clone(),
            failed["result"].clone(),
            failed["problem"].clone()
        ),
        (json!(false), json!("failed"), json!("offline"))
    );
}

/// Every call a scripted Live got: its instance and commands.
type Calls = Arc<Mutex<Vec<(String, Vec<Value>)>>>;

/// A Live whose answers are scripted per call; every call is kept.
fn scripted(answers: Vec<Result<Vec<Value>, String>>) -> (Lom, Calls) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let answers = Arc::new(Mutex::new(answers.into_iter()));
    let kept = Arc::clone(&calls);
    let lom: Lom = Arc::new(move |instance: &str, commands: Vec<Value>| {
        kept.lock().unwrap().push((instance.to_string(), commands));
        let answer = answers.lock().unwrap().next().expect("an answer per call");
        Box::pin(async move { answer })
    });
    (lom, calls)
}

fn listen_rack() -> Vec<Value> {
    answer(
        rack_at("live_set tracks 1 devices 2"),
        not_found("return_tracks"),
        "Main",
        ok(json!([])),
    )
}

fn no_listen_rack() -> Vec<Value> {
    answer(
        ok(json!([])),
        not_found("return_tracks"),
        "Main",
        ok(json!([])),
    )
}

#[tokio::test]
async fn a_switch_finds_the_rack_then_writes_it() {
    let (lom, calls) = scripted(vec![Ok(listen_rack()), Ok(vec![ok(Value::Null)])]);
    assert_eq!(
        switch(&lom, &sw("band", "Vox 1", true)).await,
        Outcome::Done
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0], ("band".to_string(), read_commands("Vox 1")));
    assert_eq!(
        calls[1],
        (
            "band".to_string(),
            write_commands("live_set tracks 1 devices 2", true)
        )
    );
}

#[tokio::test]
async fn a_switch_with_no_rack_or_no_answer_writes_nothing() {
    let (lom, calls) = scripted(vec![Ok(no_listen_rack())]);
    assert_eq!(
        switch(&lom, &sw("band", "Vox 1", true)).await,
        Outcome::Missing(NO_RACK.into())
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    let (lom, calls) = scripted(vec![Err("offline".into())]);
    assert_eq!(
        switch(&lom, &sw("band", "Vox 1", false)).await,
        Outcome::Failed("offline".into())
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
    let (lom, _) = scripted(vec![Ok(listen_rack()), Err("no result within 3 s".into())]);
    assert_eq!(
        switch(&lom, &sw("band", "Vox 1", false)).await,
        Outcome::Failed("no result within 3 s".into())
    );
}

#[tokio::test]
async fn the_task_switches_in_order_records_each_and_answers_its_waiter() {
    let (lom, calls) = scripted(vec![
        Ok(listen_rack()),
        Ok(vec![ok(Value::Null)]),
        Ok(no_listen_rack()),
    ]);
    let (events, records) = EventLog::channel(16);
    let (tx, rx) = mpsc::unbounded_channel();
    let (done, outcome) = oneshot::channel();
    tx.send(Job {
        switch: sw("band", "Keys", true),
        done: Some(done),
    })
    .unwrap();
    tx.send(Job {
        switch: sw("band", "Vox 1", false),
        done: None,
    })
    .unwrap();
    drop(tx);
    run(rx, lom, events).await;
    assert_eq!(outcome.await.unwrap(), Outcome::Done);
    let reads: Vec<Value> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.1[0].clone())
        .collect();
    assert_eq!(reads[0], read_commands("Keys")[0]);
    assert_eq!(reads[2], read_commands("Vox 1")[0]);
    let got: Vec<Value> = records.try_iter().collect();
    assert_eq!(got.len(), 2);
    assert_eq!(
        (
            got[0]["what"].clone(),
            got[0]["track"].clone(),
            got[0]["result"].clone()
        ),
        (json!("rack"), json!("Keys"), json!("done"))
    );
    assert_eq!(
        (
            got[1]["track"].clone(),
            got[1]["on"].clone(),
            got[1]["result"].clone()
        ),
        (json!("Vox 1"), json!(false), json!("missing"))
    );
}
