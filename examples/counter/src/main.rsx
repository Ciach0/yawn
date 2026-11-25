component App() {
    let count = signal(0);

    view {
        <main class="card">
            <p class="count">{count}</p>
            <div class="row">
                <button class="btn" on:click={move || count.update(|n| *n -= 1)}>"−"</button>
                <button class="btn primary" on:click={move || count.update(|n| *n += 1)}>"+"</button>
            </div>
        </main>
    }
}
