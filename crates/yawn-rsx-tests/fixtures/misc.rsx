use std::cell::{Cell, RefCell};

thread_local! {
    pub static HANDLES: RefCell<Option<(NodeRef, NodeRef, Sig<bool>)>> = const { RefCell::new(None) };
    pub static PROBE_RUNS: Cell<usize> = const { Cell::new(0) };
    pub static PROBE_ALIVE: Cell<usize> = const { Cell::new(0) };
}

/// Wraps whatever is placed between its tags.
component Card(title: &'static str, children: Children) {
    view {
        <section class="card">
            <h2>"{title}"</h2>
            <slot />
        </section>
    }
}

/// Counts effect runs and drops, to check that branches dispose what they created.
component Probe(tick: Sig<u32>) {
    effect(move || {
        tick.get();
        PROBE_RUNS.with(|r| r.set(r.get() + 1));
    });
    on_cleanup(|| PROBE_ALIVE.with(|a| a.set(a.get() - 1)));
    PROBE_ALIVE.with(|a| a.set(a.get() + 1));
    view {
        <i class="probe" on:click={move || tick.update(|t| *t += 1)}>"probe {tick}"</i>
    }
}

component Panel() {
    let n = signal(0);
    view {
        <Card title="Panel">
            <p class="inner">"inside {n}"</p>
            <button on:click={move || n.update(|v| *v += 1)}>"more"</button>
        </Card>
    }
}

component Toggle() {
    let on = signal(true);
    let tick = signal(0u32);
    view {
        <button class="toggle" on:click={move || on.update(|v| *v = !*v)}>"toggle"</button>
        <button class="tick" on:click={move || tick.update(|t| *t += 1)}>"tick"</button>
        if on.get() {
            <Probe tick={tick} />
        }
    }
}

component Form() {
    let name = signal(String::new());
    let clicks = signal(0u32);
    let level = signal(0u8);
    view {
        <input class="name" prop:value={name.get()} placeholder="your name" on:input={move |e: Event| name.set(e.value)} />
        <p class="hello" title="hello {name}">"Hello, {name}!"</p>
        <button class="go" disabled={name.get().is_empty()} on:click={move || clicks.update(|c| *c += 1)}>"go"</button>
        <span class="clicks" data-clicks={clicks}>"{clicks}"</span>
        <button class="up" on:click={move || level.update(|l| *l += 1)}>"up"</button>
        if level.get() == 0 {
            <b class="lv">"zero"</b>
        } else if level.get() == 1 {
            <b class="lv">"one"</b>
        } else {
            <b class="lv">"many"</b>
        }
        <p class="fmt">"{level:03}|{{literal}}"</p>
        if let Some(n) = (level.get() > 0).then(|| level.get()) {
            <u class="got">"got {n}"</u>
            <button class="inc" on:click={move || level.update(|l| *l += 1)}>"inc {n}"</button>
        } else {
            <u class="got">"nothing"</u>
        }
    }
}

#[derive(Clone)]
pub struct Item {
    pub id: u32,
    pub label: String,
    pub done: Sig<bool>,
}

component Items() {
    let next = signal(4u32);
    let items = signal_always(
        (1..4).map(|id| Item { id, label: format!("item {id}"), done: signal(false) }).collect::<Vec<_>>(),
    );
    view {
        <button class="add" on:click={move || {
            let id = next.peek();
            next.set(id + 1);
            // Signals are created before `update`: its closure runs while the runtime is busy.
            let item = Item { id, label: format!("item {id}"), done: signal(false) };
            items.update(|l| l.push(item));
        }}>"add"</button>
        <button class="rev" on:click={move || items.update(|l| l.reverse())}>"reverse"</button>
        <button class="shuffle" on:click={move || items.update(|l| { let n = l.len(); if n > 2 { l.swap(0, n - 1); } })}>"swap ends"</button>
        <ul>
            for item in items.get() key item.id {
                <li class:done={item.done.get()} data-id={item.id}>
                    <span>"{item.label}"</span>
                    <button class="toggle" on:click={move || item.done.update(|d| *d = !*d)}>"done"</button>
                    <button class="rm" on:click={move || items.update(|l| l.retain(|x| x.id != item.id))}>"x"</button>
                </li>
            }
        </ul>
    }
}

component Grid() {
    let rows = signal_always(vec![(1u32, vec!["a", "b"]), (2, vec!["c"])]);
    view {
        for (id, cells) in rows.get() key *id {
            <div class="r" data-row={id}>
                for cell in cells key *cell {
                    <b>"{id}{cell}"</b>
                }
            </div>
        }
    }
}

/// Two named slots and the default one; the body slot sits in an `if`, so it is built each time
/// the box opens.
component Folding(head: Children, body: Children, children: Children) {
    let open = signal(false);
    view {
        <div class="fold">
            <button class="head" on:click={move || open.update(|o| *o = !*o)}>
                <slot name="head" />
            </button>
            if open.get() {
                <div class="body">
                    <slot name="body" />
                </div>
            }
            <footer>
                <slot />
            </footer>
        </div>
    }
}

component FoldingUse() {
    let n = signal(0);
    view {
        <Folding>
            <b slot="head">"Title"</b>
            <template slot="body">
                <p class="one">"first {n}"</p>
                <p class="two">"second"</p>
            </template>
            <button class="bump" on:click={move || n.update(|v| *v += 1)}>"bump"</button>
        </Folding>
    }
}

// A `Class` prop takes a literal, a built string or a signal, and follows the signal.
component Badge(class: Class) {
    view {
        <i class="{class} badge">"b"</i>
    }
}

component BadgeUse() {
    let tone = signal("red".to_string());
    view {
        <Badge class="plain" />
        <Badge class="built {1 + 1}" />
        <Badge class={tone.into()} />
        <button class="flip" on:click={move || tone.set("blue".to_string())}>"flip"</button>
    }
}

/// Builds an input and a conditional link with `ref`, and leaves the handles in `HANDLES`.
component Handles() {
    let show = signal(true);
    let input = node_ref();
    let link = node_ref();
    HANDLES.with(|h| *h.borrow_mut() = Some((input, link, show)));
    view {
        <div>
            <input ref={input} id="in" class="field" />
            if show.get() {
                <a ref={link} id="lnk" href="/x">"x"</a>
            }
        </div>
    }
}
