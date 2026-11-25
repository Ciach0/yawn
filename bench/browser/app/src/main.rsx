use crate::bench::{build, Item};
use crate::cards::Cards;
use crate::row::BenchRow;
use yawn::ui::{signal, signal_always};

mod bench;

component App() {
    let rows = signal_always(Vec::<Item>::new());
    let selected = signal(0u32);
    let tick = signal(0u32);
    let ping = signal(0u32);
    let cards = signal(false);

    view {
        <div id="controls">
            <button id="ping" on:click={move || ping.update(|p| *p += 1)}>"ping"</button>
            <span id="pingv">"{ping}"</span>
            <button id="create1k" on:click={move || rows.set(build(1000))}>"create 1k"</button>
            <button id="create10k" on:click={move || rows.set(build(10000))}>"create 10k"</button>
            <button id="append1k" on:click={move || { let more = build(1000); rows.update(|v| v.extend(more)); }}>"append 1k"</button>
            <button id="update10th" on:click={move || { let labels: Vec<_> = rows.with(|v| v.iter().step_by(10).map(|r| r.label).collect()); for l in labels { l.update(|t| t.push_str(" !!!")); } }}>"update every 10th"</button>
            <button id="select" on:click={move || selected.set(rows.with(|v| v[2].id))}>"select"</button>
            <button id="swap" on:click={move || rows.update(|v| v.swap(1, 998))}>"swap"</button>
            <button id="remove" on:click={move || rows.update(|v| { v.remove(1); })}>"remove"</button>
            <button id="clear" on:click={move || rows.set(Vec::new())}>"clear"</button>
            <button id="fan" on:click={move || tick.update(|t| *t += 1)}>"fan"</button>
            <button id="cards" on:click={move || cards.update(|c| *c = !*c)}>"cards"</button>
        </div>
        <div id="rows">
            for item in rows.get() key item.id {
                <BenchRow item={item} selected={selected} />
            }
        </div>
        <div id="fanout">
            for _i in 0..1000usize key *_i {
                <span class="fan">"{tick}"</span>
            }
        </div>
        if cards.get() {
            <Cards />
        }
    }
}
