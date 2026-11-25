use crate::bench::Item;
use yawn::ui::Sig;

// One row: a handful of static elements around two dynamic texts and a class that follows the
// selection. Built once per id.
component BenchRow(item: Item, selected: Sig<u32>) {
    let id = item.id;
    let label = item.label;

    view {
        <div class="row" class:danger={selected.get() == id}>
            <span class="c1">"{id}"</span>
            <span class="c2"><a class="lbl" on:click={move || selected.set(id)}>"{label}"</a></span>
            <span class="c3"><a class="rm"><i class="icon" aria-hidden="true"></i></a></span>
            <span class="c4"></span>
        </div>
    }
}
