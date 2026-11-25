use crate::stepper::Stepper;
use std::cell::Cell;

thread_local! {
    pub static TITLE_RUNS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, PartialEq)]
pub struct Entry {
    pub id: u32,
    pub text: String,
}

component Counter() {
    let count = signal(0i32);
    let step = signal(1i32);
    let double = memo(move || count.get() * 2);
    let parity = memo(move || if count.get() % 2 == 0 { "even" } else { "odd" });
    let log = signal(Vec::<Entry>::new());
    let next_id = signal(1u32);

    effect(move || {
        TITLE_RUNS.with(|r| r.set(r.get() + 1));
        set_title(&format!("Count: {count}"));
    });

    let change = move |sign: i32| {
        let s = step.peek();
        count.update(|c| *c += sign * s);
        let id = next_id.peek();
        next_id.set(id + 1);
        let text = format!("now {}", count.peek());
        log.update(|l| l.push(Entry { id, text }));
    };

    view {
        <main class="card">
            <h1>"Counter"</h1>
            <p class="count" class:big={count.get() >= 10}>{count}</p>
            <p class="meta">"double = {double}, {parity}"</p>
            <div class="row">
                <button class="btn minus" on:click={move || change(-1)}>"−"</button>
                <button class="btn plus primary" on:click={move || change(1)}>"+"</button>
                <button class="btn reset" on:click={move || count.set(0)}>"reset"</button>
            </div>
            <Stepper step={step} />
            if count.get() >= 10 {
                <p class="notice">"that is a big number"</p>
            } else {
                <p class="notice small">"keep going"</p>
            }
            <ul class="log">
                for entry in log.get() key entry.id {
                    <li>
                        <span>"#{entry.id}: {entry.text}"</span>
                        <button class="x" on:click={move || log.update(|l| l.retain(|e| e.id != entry.id))}>"x"</button>
                    </li>
                }
            </ul>
        </main>
    }
}
