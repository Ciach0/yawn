// The second component of the counter example: lives in its own file and is imported.
component Stepper(step: Sig<i32>) {
    let label = memo(move || format!("step {}", step.get()));

    view {
        <div class="row stepper">
            <span class="meta">{label}</span>
            <button class="btn small double" on:click={move || step.update(|s| *s = (*s * 2).min(64))}>"step ×2"</button>
            <button class="btn small one" on:click={move || step.set(1)}>"step 1"</button>
        </div>
    }
}
