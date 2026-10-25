use crate::node::{META_FILE, NODE_DIR};
use crate::taskrec::{
    self, Answer, Approval, Assertion, Assumption, Grant, Grants, Links, PlanMeta, Question, Requests, SCHEMA, State, Step, TaskRecord,
    Verdict, VerdictItem,
};
use chrono::{DateTime, Duration, Utc};
use std::fmt::Write as _;
use std::io;
use std::path::Path;

pub const PIN_ENV: &str = "FABR_SAMPLE_NOW";
const PINNED_ROOT: &str = "<SAMPLE>/home";
const PINNED_PID: u32 = 41872;
const DEAD_PID: u32 = 99999;
const LIVE_INTERVAL_S: u64 = 1200;
const M: i64 = 60;

const RULE_PAY: &str = "Never fill payment details; stop and hand the last step to me.";
const RULE_OVER: &str = "Ask before anything over $100.";
const RULE_CARD: &str = "Personal card only under here.";
const JAPAN_FACTS: [&str; 3] = [
    "Dates: 21 Dec to 30 Dec 2026.",
    "Two travellers.",
    "Arrive at Kansai International (KIX) on 21 Dec; Kyoto 23 to 26 Dec, Tokyo 26 to 30 Dec.",
];
const WORK_FACTS: [&str; 3] = [
    "CI is GitHub Actions; the main workflow is .github/workflows/ci.yml.",
    "Releases are vX.Y.Z tags on main; a tag push runs release.yml.",
    "Stripe keys are rotated in the dashboard and stored as the STRIPE_SECRET_KEY repository secret.",
];

struct Clock {
    now: DateTime<Utc>,
    pinned: bool,
}

impl Clock {
    fn new() -> Clock {
        match std::env::var(PIN_ENV).ok().and_then(|v| DateTime::parse_from_rfc3339(&v).ok()) {
            Some(t) => Clock { now: t.with_timezone(&Utc), pinned: true },
            None => Clock { now: Utc::now(), pinned: false },
        }
    }

    fn at(&self, secs: i64) -> String {
        (self.now + Duration::seconds(secs)).format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    fn day(&self, days: i64) -> String {
        (self.now + Duration::days(days)).format("%Y-%m-%d").to_string()
    }
}

struct Agent {
    dead: bool,
    started: i64,
    step_at: i64,
    beat: i64,
    activity: &'static str,
    usage: Vec<(&'static str, u32, u32, u32, f64)>,
}

struct Sample {
    node: &'static str,
    rec: TaskRecord,
    agent: Option<Agent>,
}

struct NodeSpec {
    rel: &'static str,
    name: &'static str,
    description: &'static str,
    color: Option<&'static str>,
}

const NODES: [NodeSpec; 7] = [
    NodeSpec { rel: "", name: "~", description: "Everything, one tree", color: None },
    NodeSpec { rel: "work", name: "work", description: "Day job", color: None },
    NodeSpec { rel: "calhacks-13", name: "calhacks-13", description: "Hackathon weekend", color: Some("indigo") },
    NodeSpec { rel: "personal", name: "personal", description: "Life outside work", color: Some("orange") },
    NodeSpec {
        rel: "personal/japan-trip-2026",
        name: "japan-trip-2026",
        description: "Kyoto and Tokyo, late December",
        color: Some("red"),
    },
    NodeSpec { rel: "personal/climbing", name: "climbing", description: "Gym and gear", color: Some("yellow") },
    NodeSpec { rel: "personal/dinner-party", name: "dinner-party", description: "Saturday dinner for six", color: Some("pink") },
];

fn s(v: &str) -> String {
    v.to_string()
}

fn steps(items: &[&str]) -> String {
    items.iter().enumerate().map(|(i, t)| format!("{}. {t}", i + 1)).collect::<Vec<_>>().join("\n")
}

fn log(c: &Clock, items: &[(i64, &str, &str)]) -> String {
    items.iter().map(|(t, step, msg)| format!("- {} {step}: {msg}\n", c.at(*t))).collect()
}

fn gathered(own: &str, extra: &[String]) -> String {
    let mut out = format!("{own}\n\n### Gathered\n- Rules (home): {RULE_PAY} {RULE_OVER}");
    for line in extra {
        out.push_str("\n- ");
        out.push_str(line);
    }
    out
}

fn base(c: &Clock, title: &str, created: i64, state: State) -> TaskRecord {
    let at = c.at(created);
    TaskRecord {
        schema: SCHEMA,
        id: format!("{}-{}", &at[..10], crate::fabric::slugify(title)),
        source: None,
        title: s(title),
        due: None,
        pri: 0,
        state,
        step: None,
        waiting: false,
        paused: false,
        suspended: false,
        suspend_until: None,
        created_at: at.clone(),
        updated_at: at,
        requests: Requests { delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: None },
        question: None,
        answers: Vec::new(),
        grants: Grants { requested: Vec::new(), granted: Vec::new() },
        plan_meta: None,
        assertions: Vec::new(),
        approval: None,
        verdict: None,
        cost_usd: 0.0,
        links: Links { redelegated_from: None, redelegated_to: None },
        by: Default::default(),
        needs: None,
        context: String::new(),
        spec: String::new(),
        success: String::new(),
        plan: String::new(),
        log: String::new(),
    }
}

fn reminder(c: &Clock, node: &'static str, title: &str, created: i64, due_days: i64, pri: u8, context: &str) -> Sample {
    let mut r = base(c, title, created, State::Reminder);
    r.due = Some(c.day(due_days));
    r.pri = pri;
    r.context = s(context);
    Sample { node, rec: r, agent: None }
}

fn assertions(items: &[&str]) -> Vec<Assertion> {
    items.iter().enumerate().map(|(i, t)| Assertion { id: format!("a{}", i + 1), text: s(t) }).collect()
}

fn meta(reversibility: &str, tools: &[&str], assumptions: &[(&str, &str)]) -> PlanMeta {
    PlanMeta {
        reversibility: Some(s(reversibility)),
        tools: tools.iter().map(|t| s(t)).collect(),
        assumptions: assumptions.iter().map(|(id, text)| Assumption { rule_id: s(id), text: s(text) }).collect(),
        checks: Vec::new(),
        requires: Default::default(),
        bound_actions: Vec::new(),
    }
}

fn approved(c: &Clock, rec: &mut TaskRecord, at: i64) {
    rec.approval = Some(Approval { status: s("approved"), by: Some(s("gui")), at: Some(c.at(at)), note: None, digest: None });
    let digest = taskrec::approval_digest(rec);
    if let Some(a) = rec.approval.as_mut() {
        a.digest = Some(digest);
    }
}

fn question(id: &str, kind: &str, text: &str, options: Option<&[&str]>, asked_at: String) -> Question {
    Question {
        id: s(id),
        kind: s(kind),
        text: s(text),
        options: options.map(|o| o.iter().map(|x| s(x)).collect()),
        prior: None,
        fields: None,
        asked_at: Some(asked_at),
        answer: None,
    }
}

fn finish(rec: &mut TaskRecord) {
    let last = rec.log.lines().last().and_then(|l| l.strip_prefix("- ")).and_then(|l| l.split(' ').next());
    rec.updated_at = last.map_or_else(|| rec.created_at.clone(), s);
}

fn ci(c: &Clock) -> Sample {
    let mut r = base(c, "fix the flaky CI", -200 * M, State::InProgress);
    r.step = Some(Step::Execute);
    r.due = Some(c.day(2));
    r.pri = 2;
    r.cost_usd = 0.4;
    r.context = gathered(
        "The ci.yml run fails about one time in five on test_checkout_retry. Find out why and make it stable.",
        &[format!("Folder knowledge (work): {} {}", WORK_FACTS[0], WORK_FACTS[1])],
    );
    r.spec = s(
        "Find the cause of the intermittent failure in test_checkout_retry, fix it in the test or the code under test, and open a draft pull request. Do not merge and do not touch the release workflow.",
    );
    r.success = s("test_checkout_retry passes 50 runs in a row and a draft pull request is open.");
    r.plan = steps(&[
        "List the last 30 ci.yml runs and read the failing logs.",
        "Reproduce the failure locally by repeating the test.",
        "Fix the race between the test and the mock payment server.",
        "Repeat the test 50 times, then open a draft pull request.",
    ]);
    r.plan_meta = Some(meta("reversible", &["shell", "github"], &[("ci-github-actions", "CI is GitHub Actions, workflow ci.yml")]));
    r.assertions = assertions(&[
        "tests/test_checkout.py passes 50 repeated runs",
        "No workflow file under .github/workflows is changed",
        "A draft pull request is open and not merged",
    ]);
    r.log = log(
        c,
        &[
            (-200 * M, "gather", "delegate accepted, folder lock taken"),
            (-199 * M, "gather", "context bundle written"),
            (-196 * M, "define", "spec and 3 assertions written, 0 grants requested"),
            (-190 * M, "plan", "plan written, reversible, tools: shell, github"),
            (-189 * M, "approve", "approval pending, message sent"),
            (-170 * M, "approve", "approved by gui"),
            (-165 * M, "execute", "started, registry entry created"),
            (-160 * M, "execute", "tool shell: list the last 30 ci.yml runs"),
            (-152 * M, "execute", "tool shell: read the failed log of run 11482"),
            (-140 * M, "execute", "tool files: read tests/test_checkout.py"),
            (-128 * M, "execute", "tool shell: pytest tests/test_checkout.py -k retry --count 20, 3 failed"),
            (-115 * M, "execute", "tool files: edited tests/test_checkout.py to wait for the mock server"),
            (-102 * M, "execute", "tool shell: pytest tests/test_checkout.py --count 50, 50 passed"),
            (-6 * M, "execute", "tool shell: pushed branch fix/flaky-checkout-test"),
            (-3 * M, "execute", "tool github: opening a draft pull request"),
        ],
    );
    approved(c, &mut r, -170 * M);
    finish(&mut r);
    let usage = vec![("define", 2, 4210, 690, 0.04), ("plan", 2, 3100, 820, 0.07), ("execute", 9, 61200, 4100, 0.29)];
    Sample {
        node: "work",
        rec: r,
        agent: Some(Agent { dead: false, started: -165 * M, step_at: -165 * M, beat: -4, activity: "tool", usage }),
    }
}

fn stripe(c: &Clock) -> Sample {
    let mut r = base(c, "rotate the expiring Stripe key", -300 * M, State::InProgress);
    r.step = Some(Step::Execute);
    r.paused = true;
    r.due = Some(c.day(1));
    r.pri = 3;
    r.cost_usd = 0.21;
    r.context = gathered(
        "The live Stripe secret key expires tomorrow. Rotate it before the deploy window.",
        &[format!("Folder knowledge (work): {}", WORK_FACTS[2])],
    );
    r.spec = s(
        "Create a replacement restricted key in the Stripe dashboard, store it as the STRIPE_SECRET_KEY repository secret, redeploy staging, run one test charge, then revoke the old key. Never print the key value in a log.",
    );
    r.success = s("Staging runs on the new key, a test charge succeeds, and the old key is revoked.");
    r.plan = steps(&[
        "Create a restricted key named ci-2026-10 in the Stripe dashboard.",
        "Store it as the STRIPE_SECRET_KEY repository secret.",
        "Redeploy staging and run one test charge.",
        "Revoke the old key.",
    ]);
    r.plan_meta = Some(meta(
        "irreversible",
        &["browser", "github"],
        &[("stripe-secret-repo", "The key lives only in the STRIPE_SECRET_KEY repository secret")],
    ));
    r.assertions = assertions(&[
        "The STRIPE_SECRET_KEY secret was updated after the new key was created",
        "A test charge on staging succeeded with the new key",
        "No key value appears in any log or record",
    ]);
    r.log = log(
        c,
        &[
            (-300 * M, "gather", "delegate accepted, folder lock taken"),
            (-299 * M, "gather", "context bundle written"),
            (-294 * M, "define", "spec and 3 assertions written, 0 grants requested"),
            (-286 * M, "plan", "plan written, irreversible, tools: browser, github"),
            (-285 * M, "approve", "approval pending, message sent"),
            (-120 * M, "approve", "approved by gui"),
            (-118 * M, "execute", "started, registry entry created"),
            (-112 * M, "execute", "tool browser: opened the Stripe dashboard, API keys page"),
            (-104 * M, "execute", "tool browser: created restricted key ci-2026-10"),
            (-34 * M, "execute", "tool github: updated the STRIPE_SECRET_KEY secret, ok"),
            (-30 * M, "execute", "paused by user"),
        ],
    );
    approved(c, &mut r, -120 * M);
    finish(&mut r);
    let usage = vec![("define", 2, 3800, 640, 0.03), ("plan", 2, 3300, 790, 0.06), ("execute", 5, 28100, 2200, 0.12)];
    Sample {
        node: "work",
        rec: r,
        agent: Some(Agent { dead: false, started: -118 * M, step_at: -118 * M, beat: -5, activity: "paused", usage }),
    }
}

fn devpost(c: &Clock) -> Sample {
    let mut r = base(c, "submit the Devpost", -95 * M, State::Task);
    r.step = Some(Step::Define);
    r.waiting = true;
    r.due = Some(format!("{}T23:00", c.day(0)));
    r.pri = 3;
    r.cost_usd = 0.03;
    r.context = gathered("Submit fabr to CalHacks 13 on Devpost before the deadline: name, tagline, demo video and repo link.", &[]);
    r.question = Some(question("q1", "open", "Which track are we submitting to: AI, Social Good, or Dev Tools?", None, c.at(-88 * M)));
    r.log = log(
        c,
        &[
            (-95 * M, "gather", "delegate accepted, folder lock taken"),
            (-94 * M, "gather", "context bundle written"),
            (-88 * M, "define", "asked q1"),
        ],
    );
    finish(&mut r);
    let usage = vec![("define", 2, 3100, 410, 0.03)];
    Sample {
        node: "calhacks-13",
        rec: r,
        agent: Some(Agent { dead: false, started: -95 * M, step_at: -88 * M, beat: -6, activity: "parked", usage }),
    }
}

fn ryokan(c: &Clock) -> Sample {
    let mut r = base(c, "book the Kyoto ryokan", -240 * M, State::Defined);
    r.step = Some(Step::Approve);
    r.waiting = true;
    r.due = Some(c.day(11));
    r.pri = 2;
    r.cost_usd = 0.12;
    r.context = gathered(
        "Three nights in a ryokan in Kyoto for the two of us, half board, a quiet room if they have one.",
        &[format!("Rules (personal): {RULE_CARD}"), format!("Folder knowledge (japan-trip-2026): {}", JAPAN_FACTS.join(" "))],
    );
    r.spec = s(
        "Book a twin tatami room for 2 guests at Sakura-an Ryokan in Higashiyama, Kyoto, from 23 Dec to 26 Dec 2026 (3 nights), half board, through sakura-an.example-ryokan.test. Fill the guest form with the granted passport name and home address. Stop at the payment step and hand it to the human; the total is near 1,240 USD, so this waits for approval.",
    );
    r.success = s("The room is held for 23 to 26 Dec, the guest form is filled in, and I only have to pay with the personal card.");
    r.plan = steps(&[
        "Open sakura-an.example-ryokan.test and check twin tatami rooms for 23 to 26 Dec.",
        "Choose the half-board rate for 2 guests.",
        "Fill the guest form with name.passport and address.line_1 from the data bank.",
        "Add the arrival note: coming from Osaka, around 15:00.",
        "Stop at the payment page and hand payment to the human.",
    ]);
    r.plan_meta = Some(meta(
        "irreversible",
        &["browser"],
        &[
            ("personal-card-only", "Personal card only under here (rule from personal); payment is handed over"),
            ("ask-over-100", "The stay costs more than 100 USD, so approval comes first"),
        ],
    ));
    r.assertions = assertions(&[
        "The booking is a twin tatami room for 2 guests, 23 to 26 Dec 2026, half board",
        "The guest form is filled from granted fields only",
        "The run stops on the payment page and no payment field is filled",
    ]);
    r.grants.requested = vec![s("name.passport"), s("address.line_1")];
    r.approval = Some(Approval { status: s("pending"), by: None, at: None, note: None, digest: None });
    r.log = log(
        c,
        &[
            (-240 * M, "gather", "delegate accepted, folder lock taken"),
            (-239 * M, "gather", "context bundle written"),
            (-225 * M, "define", "spec and 3 assertions written, 2 grants requested"),
            (-215 * M, "plan", "plan written, irreversible, tools: browser"),
            (-214 * M, "approve", "approval pending, message sent"),
        ],
    );
    finish(&mut r);
    let usage = vec![("define", 3, 6100, 1180, 0.05), ("plan", 2, 4400, 1500, 0.07)];
    Sample {
        node: "personal/japan-trip-2026",
        rec: r,
        agent: Some(Agent { dead: false, started: -240 * M, step_at: -214 * M, beat: -3, activity: "parked", usage }),
    }
}

fn shinkansen(c: &Clock) -> Sample {
    let mut r = base(c, "reserve the Shinkansen seats", -1500 * M, State::Succeeded);
    r.due = Some(c.day(-1));
    r.pri = 1;
    r.cost_usd = 0.31;
    r.context = gathered(
        "Two reserved seats Kyoto to Tokyo on the day we change cities.",
        &[format!("Rules (personal): {RULE_CARD}"), format!("Folder knowledge (japan-trip-2026): {}", JAPAN_FACTS.join(" "))],
    );
    r.spec = s(
        "Reserve 2 seats together on the Nozomi from Kyoto to Tokyo on 26 Dec 2026 at 10:30, ordinary car, window side, at shinkansen.example-rail.test. Fill the traveller names from the granted fields. Stop at payment.",
    );
    r.success = s("Two seats are reserved Kyoto to Tokyo on 26 Dec and I only have to pay.");
    r.plan = steps(&[
        "Open shinkansen.example-rail.test and search Kyoto to Tokyo on 26 Dec at 10:30.",
        "Pick the Nozomi and ask which seat side to take.",
        "Select 2 seats together, window side.",
        "Fill the traveller names from name.first and name.last.",
        "Stop at the payment page.",
    ]);
    r.plan_meta = Some(meta(
        "irreversible",
        &["browser"],
        &[("personal-card-only", "Personal card only under here (rule from personal); payment is handed over")],
    ));
    r.assertions = assertions(&[
        "The reservation is for 2 seats, Kyoto to Tokyo, 26 Dec 2026, 10:30",
        "Both seats are together and one is a window seat",
        "The run stops on the payment page and no payment field is filled",
    ]);
    r.grants = Grants {
        requested: vec![s("name.first"), s("name.last")],
        granted: vec![Grant { field: s("name.first"), scope: s("task") }, Grant { field: s("name.last"), scope: s("task") }],
    };
    let mut q = question("q1", "choice", "Window or aisle?", Some(&["window", "aisle", "no preference"]), c.at(-1495 * M));
    q.answer = Some(Answer { value: Some(s("window")), replacement: None, granted: None, by: Some(s("gui")), at: Some(c.at(-1470 * M)) });
    r.answers = vec![q];
    r.log = log(
        c,
        &[
            (-1500 * M, "gather", "delegate accepted, folder lock taken"),
            (-1499 * M, "gather", "context bundle written"),
            (-1495 * M, "define", "asked q1"),
            (-1470 * M, "define", "q1 answered by gui: window"),
            (-1465 * M, "define", "spec and 3 assertions written, 2 grants requested"),
            (-1455 * M, "plan", "plan written, irreversible, tools: browser"),
            (-1454 * M, "approve", "approval pending, message sent"),
            (-1440 * M, "approve", "approved by gui"),
            (-1430 * M, "execute", "started, registry entry created"),
            (-1424 * M, "execute", "tool browser: searched Kyoto to Tokyo, 26 Dec, 10:30"),
            (-1420 * M, "execute", "tool browser: chose the Nozomi, 2 seats together on the window side"),
            (-1418 * M, "execute", "tool browser: filled name.first and name.last"),
            (-1415 * M, "execute", "stopped on the payment page, payment handed to the human"),
            (-1395 * M, "evaluate", "a1 pass"),
            (-1395 * M + 1, "evaluate", "a2 pass"),
            (-1395 * M + 2, "evaluate", "a3 pass"),
            (-1394 * M, "evaluate", "verdict pass, state succeeded"),
            (-1393 * M, "learn", "saved a project preference: window seat on trains"),
            (-1392 * M, "notify", "notice sent"),
        ],
    );
    approved(c, &mut r, -1440 * M);
    let item = |id: &str, evidence: &str| VerdictItem { id: s(id), pass: true, evidence: s(evidence) };
    r.verdict = Some(Verdict {
        result: s("pass"),
        at: c.at(-1394 * M),
        items: vec![
            item("a1", "the confirmation page shows 2 seats, Kyoto to Tokyo, Sat 26 Dec 2026, 10:30"),
            item("a2", "seats 7A and 7B, 7A is a window seat"),
            item("a3", "audit.log shows 2 fills (name.first, name.last) and the last page is the payment page, empty"),
        ],
    });
    finish(&mut r);
    Sample { node: "personal/japan-trip-2026", rec: r, agent: None }
}

fn gym(c: &Clock) -> Sample {
    let mut r = base(c, "renew the gym membership", -180 * M, State::InProgress);
    r.step = Some(Step::Execute);
    r.due = Some(c.day(3));
    r.pri = 1;
    r.cost_usd = 0.18;
    r.context =
        gathered("The Bayline Boulders membership runs out this week. Renew the same plan.", &[format!("Rules (personal): {RULE_CARD}")]);
    r.spec = s(
        "Sign in to bayline-boulders.example-gym.test with the saved browser session, open Membership, renew the same plan (Monthly Unlimited, 89 USD), and stop at payment.",
    );
    r.success = s("The renewal is in the cart for the same plan and I only have to pay.");
    r.plan = steps(&[
        "Open bayline-boulders.example-gym.test with the saved browser session.",
        "Open Membership and choose Renew.",
        "Keep the same plan, Monthly Unlimited.",
        "Stop at the payment page and hand payment to the human.",
    ]);
    r.plan_meta = Some(meta(
        "reversible",
        &["browser"],
        &[("personal-card-only", "Personal card only under here (rule from personal); payment is handed over")],
    ));
    r.assertions = assertions(&[
        "The renewal is for Monthly Unlimited, 89 USD",
        "No account setting was changed",
        "The run stops on the payment page and no payment field is filled",
    ]);
    r.log = log(
        c,
        &[
            (-180 * M, "gather", "delegate accepted, folder lock taken"),
            (-179 * M, "gather", "context bundle written"),
            (-175 * M, "define", "spec and 3 assertions written, 0 grants requested"),
            (-168 * M, "plan", "plan written, reversible, tools: browser"),
            (-167 * M, "approve", "approval pending, message sent"),
            (-150 * M, "approve", "approved by gui"),
            (-75 * M, "execute", "started, registry entry created"),
            (-72 * M, "execute", "tool browser: opened bayline-boulders.example-gym.test"),
            (-60 * M, "execute", "tool browser: signed in with the saved browser session"),
            (-48 * M, "execute", "tool browser: opened Membership, Renew"),
            (-41 * M, "execute", "tool browser: kept the plan Monthly Unlimited"),
        ],
    );
    approved(c, &mut r, -150 * M);
    finish(&mut r);
    let usage = vec![("define", 2, 3500, 520, 0.03), ("plan", 2, 2900, 700, 0.05), ("execute", 4, 21400, 1500, 0.1)];
    Sample {
        node: "personal/climbing",
        rec: r,
        agent: Some(Agent { dead: true, started: -75 * M, step_at: -75 * M, beat: -40 * M, activity: "tool", usage }),
    }
}

fn samples(c: &Clock) -> Vec<Sample> {
    vec![
        ci(c),
        stripe(c),
        devpost(c),
        reminder(c, "calhacks-13", "order a USB-C hub for the team", -60 * M, 1, 2, "Four 100 W USB-C hubs with HDMI for the team table."),
        ryokan(c),
        shinkansen(c),
        gym(c),
        reminder(c, "personal/climbing", "order new climbing shoes", -1440 * M, 6, 0, "Replace the worn La Sportiva pair, same size 42.5."),
        reminder(c, "personal/dinner-party", "send the invites", -2880 * M, 2, 1, "Text the five guests: Saturday, 7:30 pm, our place."),
        reminder(
            c,
            "personal/dinner-party",
            "order the wine",
            -2880 * M,
            9,
            0,
            "Two reds and a white to go with the lamb, under 80 USD in total.",
        ),
    ]
}

fn meta_text(n: &NodeSpec) -> String {
    let mut out = format!("name = {}\ndescription = {}\n", quote(n.name), quote(n.description));
    if let Some(color) = n.color {
        let _ = writeln!(out, "color = {}", quote(color));
    }
    if n.rel.is_empty() {
        out.push_str("show_depth = 1\n");
    }
    out
}

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

struct Place {
    prefix: String,
}

impl Place {
    fn folder(&self, node: &str) -> String {
        if node.is_empty() { self.prefix.clone() } else { format!("{}/{node}", self.prefix) }
    }

    fn task(&self, sm: &Sample) -> String {
        format!("{}/.fabric-space/tasks/{}.md", self.folder(sm.node), sm.rec.id)
    }

    fn task_titled(&self, all: &[Sample], title: &str) -> String {
        all.iter().find(|x| x.rec.title == title).map(|x| self.task(x)).unwrap_or_default()
    }
}

fn name_of<T: serde::Serialize>(v: T) -> String {
    serde_yaml::to_value(v).ok().and_then(|v| v.as_str().map(s)).unwrap_or_else(|| s("null"))
}

fn registry_text(c: &Clock, place: &Place, sm: &Sample, a: &Agent) -> String {
    let r = &sm.rec;
    let waiting = match (r.waiting, &r.question) {
        (false, _) => "null",
        (true, Some(_)) => "question",
        (true, None) => "approval",
    };
    let pid = if a.dead {
        DEAD_PID
    } else if c.pinned {
        PINNED_PID
    } else {
        std::process::id()
    };
    let interval = if a.dead { 10 } else { LIVE_INTERVAL_S };
    let mut out = String::new();
    let _ = writeln!(out, "task_path: {}", quote(&place.task(sm)));
    let _ = writeln!(out, "task_id: {}", quote(&r.id));
    let _ = writeln!(out, "folder: {}", quote(&place.folder(sm.node)));
    let _ = writeln!(out, "state: {}", name_of(r.state));
    let _ = writeln!(out, "step: {}", r.step.map_or_else(|| s("null"), name_of));
    let _ = writeln!(out, "step_at: {}", quote(&c.at(a.step_at)));
    let _ = writeln!(out, "started_at: {}", quote(&c.at(a.started)));
    let _ = writeln!(out, "heartbeat_at: {}", quote(&c.at(a.beat)));
    let _ = writeln!(out, "heartbeat_interval_s: {interval}");
    let _ = writeln!(out, "waiting: {waiting}");
    let _ = writeln!(out, "activity: {}", a.activity);
    out.push_str("retry_in_s: null\nlast_error: null\n");
    let _ = writeln!(out, "cost_usd: {}", r.cost_usd);
    out.push_str("usage:\n");
    for (name, calls, input, output, usd) in &a.usage {
        let _ = writeln!(out, "  {name}: {{calls: {calls}, input_tokens: {input}, output_tokens: {output}, usd: {usd}}}");
    }
    out.push_str("log_tail:\n");
    let all: Vec<&str> = r.log.lines().filter(|l| !l.trim().is_empty()).collect();
    for l in &all[all.len().saturating_sub(20)..] {
        let _ = writeln!(out, "  - {}", quote(l.strip_prefix("- ").unwrap_or(l)));
    }
    let q = r.question.as_ref().and_then(|q| serde_json::to_string(q).ok()).unwrap_or_else(|| s("null"));
    let _ = writeln!(out, "question: {q}");
    let _ = writeln!(out, "pid: {pid}");
    out
}

fn events_text(c: &Clock, place: &Place, all: &[Sample]) -> String {
    let mut ev = [
        (-88 * M, "waiting", place.task_titled(all, "submit the Devpost"), "one question"),
        (-214 * M, "waiting", place.task_titled(all, "book the Kyoto ryokan"), "one approval"),
        (-1392 * M, "done", place.task_titled(all, "reserve the Shinkansen seats"), "3 of 3 assertions passed"),
        (-40 * M + 31, "stuck", place.task_titled(all, "renew the gym membership"), "no heartbeat for 31 s"),
    ];
    ev.sort_by_key(|e| e.0);
    ev.iter().map(|(t, kind, path, detail)| format!("{} {kind:<7} {path} {detail}\n", c.at(*t))).collect()
}

fn put(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

fn bullets(items: &[&str]) -> String {
    items.iter().map(|i| format!("- {i}\n")).collect()
}

fn learned(c: &Clock, task: &str, id: &str, text: &str, when: &str, scope: &str) -> String {
    format!(
        "records:\n  - id: {id}\n    text: {}\n    applies_when: {}\n    scope: {scope}\n    count: 1\n    source_tasks:\n      - {task}\n    last_confirmed: {}\n",
        quote(text),
        quote(when),
        c.at(-1393 * M)
    )
}

pub fn write_sample(root: &Path) -> io::Result<usize> {
    let c = Clock::new();
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    let place = Place { prefix: if c.pinned { s(PINNED_ROOT) } else { root.display().to_string() } };
    let all = samples(&c);
    for n in &NODES {
        put(&root.join(n.rel).join(NODE_DIR).join(META_FILE), &meta_text(n))?;
    }
    let space = root.join(".fabric-space");
    for sm in &all {
        put(&root.join(sm.node).join(".fabric-space/tasks").join(format!("{}.md", sm.rec.id)), &taskrec::render(&sm.rec))?;
        if let Some(a) = &sm.agent {
            let name = taskrec::registry_filename(&sm.rec.id, &place.task(sm));
            put(&space.join("registry").join(name), &registry_text(&c, &place, sm, a))?;
        }
    }
    let backend_pid = if c.pinned { PINNED_PID } else { std::process::id() };
    let backend = format!(
        "heartbeat_at: {}\nheartbeat_interval_s: 10\npid: {backend_pid}\nstarted_at: {}\n",
        quote(&c.at(-4)),
        quote(&c.at(-300 * M))
    );
    put(&space.join("registry/_backend.yaml"), &backend)?;
    put(&space.join("events.log"), &events_text(&c, &place, &all))?;
    let settings = format!(
        "default_folder = {}\nmodel = \"claude-sonnet-5-5\"\ntelegram_user_id = 123456789\nauto_mode = false\nquestion_budget = 5\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"api_key\", \"model\", \"telegram_bot\", \"telegram_link\"]\ncompleted = true\n",
        quote(&place.prefix)
    );
    put(&space.join("settings.toml"), &settings)?;
    put(&space.join(".agents/rules.md"), &bullets(&[RULE_PAY, RULE_OVER]))?;
    put(
        &space.join(".agents/preferences.toml"),
        "language = \"en\"\ncurrency = \"USD\"\nnotice_style = \"short\"\nquiet_hours = \"22:00-07:00\"\n",
    )?;
    let shin = place.task_titled(&all, "reserve the Shinkansen seats");
    let global =
        learned(&c, &shin, "g1", "Stop on the payment page and hand payment to the human", "any checkout flow in the browser", "global");
    put(&space.join(".agents/learned.yaml"), &global)?;
    let local = learned(&c, &shin, "l1", "window seat on trains", "reserving seats on a train", "project");
    put(&root.join("personal/.fabric-space/.agents/learned.yaml"), &local)?;
    put(&root.join("personal/.fabric-space/.agents/rules.md"), &bullets(&[RULE_CARD]))?;
    put(&root.join("personal/japan-trip-2026/.fabric-space/.agents/knowledge.md"), &bullets(&JAPAN_FACTS))?;
    put(&root.join("work/.fabric-space/.agents/knowledge.md"), &bullets(&WORK_FACTS))?;
    put(
        &space.join("data-bank/policy.toml"),
        "[fields]\n\"name\" = \"always-allow\"\n\"address\" = \"always-allow\"\n\"address.line_2\" = \"always-ask\"\n\"payment\" = \"never\"\n",
    )?;
    let fill = |secs: i64, field: &str| format!("{} {shin} {field} browser:shinkansen.example-rail.test\n", c.at(secs));
    put(&space.join("data-bank/audit.log"), &(fill(-1418 * M, "name.first") + &fill(-1418 * M + 15, "name.last")))?;
    Ok(NODES.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(dir: &Path, ext: &str, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, ext, out);
            } else if p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }

    #[test]
    fn sample_has_the_demo_states() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("home");
        assert_eq!(write_sample(&root).unwrap(), 7);
        let mut files = Vec::new();
        walk(&root, "md", &mut files);
        let recs: Vec<(String, TaskRecord)> = files
            .iter()
            .filter(|p| p.to_string_lossy().contains("/tasks/"))
            .map(|p| (p.to_string_lossy().into_owned(), taskrec::parse(&std::fs::read_to_string(p).unwrap()).unwrap()))
            .collect();
        assert_eq!(recs.len(), 10);
        let by = |t: &str| &recs.iter().find(|(_, r)| r.title == t).unwrap().1;
        let ci = by("fix the flaky CI");
        assert_eq!((ci.state, ci.step, ci.paused, ci.plan.lines().count()), (State::InProgress, Some(Step::Execute), false, 4));
        let stripe = by("rotate the expiring Stripe key");
        assert!(stripe.paused && stripe.log.trim_end().ends_with("execute: paused by user"));
        let dev = by("submit the Devpost");
        assert_eq!((dev.state, dev.step, dev.waiting), (State::Task, Some(Step::Define), true));
        assert!(dev.question.as_ref().is_some_and(|q| q.answer.is_none()));
        let hub = by("order a USB-C hub for the team");
        assert_eq!((hub.state, hub.pri), (State::Reminder, 2));
        let ryo = by("book the Kyoto ryokan");
        assert_eq!((ryo.state, ryo.plan.lines().count(), ryo.assertions.len(), ryo.grants.requested.len()), (State::Defined, 5, 3, 2));
        assert_eq!(ryo.approval.as_ref().map(|a| a.status.as_str()), Some("pending"));
        let shin = by("reserve the Shinkansen seats");
        assert_eq!((shin.state, shin.answers.len()), (State::Succeeded, 1));
        assert_eq!(shin.verdict.as_ref().map(|v| (v.result.as_str(), v.items.len())), Some(("pass", 3)));
        assert_eq!(shin.answers[0].answer.as_ref().and_then(|a| a.value.as_deref()), Some("window"));
        assert_eq!(by("renew the gym membership").state, State::InProgress);
        for t in ["order new climbing shoes", "send the invites", "order the wine"] {
            assert_eq!((by(t).state, by(t).pri <= 1), (State::Reminder, true));
        }
        let mut ymls = Vec::new();
        walk(&root.join(".fabric-space/registry"), "yaml", &mut ymls);
        ymls.retain(|p| p.file_name().is_some_and(|n| n != "_backend.yaml"));
        assert_eq!(ymls.len(), 5);
        let now = Utc::now();
        let mut dead = Vec::new();
        for p in &ymls {
            let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
            let beat = DateTime::parse_from_rfc3339(doc["heartbeat_at"].as_str().unwrap()).unwrap();
            let age = (now - beat.with_timezone(&Utc)).num_seconds();
            let task_path = doc["task_path"].as_str().unwrap();
            assert!(recs.iter().any(|(path, _)| path == task_path));
            if age > 3 * doc["heartbeat_interval_s"].as_i64().unwrap() {
                dead.push(task_path.to_string());
                assert_eq!(doc["pid"].as_u64(), Some(u64::from(DEAD_PID)));
            } else {
                assert_eq!(doc["pid"].as_u64(), Some(u64::from(std::process::id())));
            }
        }
        assert_eq!(dead.len(), 1);
        assert!(dead[0].contains("renew-the-gym-membership"));
        let events = std::fs::read_to_string(root.join(".fabric-space/events.log")).unwrap();
        assert!(events.lines().any(|l| l.contains(" stuck ") && l.contains("renew-the-gym-membership")));
    }
}
