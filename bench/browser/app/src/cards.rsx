// A static-heavy block: 200 cards with about a dozen fixed elements and one dynamic number each.
// This is what most of a landing page looks like.
component Card(n: usize) {
    view {
        <div class="card">
            <h3 class="t">"Card title"</h3>
            <p class="d">"Some static description text that never changes, a sentence or two long."</p>
            <ul class="l">
                <li class="i">"One"</li>
                <li class="i">"Two"</li>
                <li class="i">"Three"</li>
            </ul>
            <div class="f">
                <a href="#open" class="b">"Open"</a>
                <span class="m">"{n}"</span>
            </div>
        </div>
    }
}

component Cards() {
    view {
        <section id="cards">
            for i in 0..200usize key *i {
                <Card n={i} />
            }
        </section>
    }
}
