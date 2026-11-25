// Props with defaults: a caller may leave them out.
component Chip(label: String, tone: String = "plain".to_string(), loud: bool = false) {
    view {
        <span class="chip {tone}" class:loud={loud}>"{label}"</span>
    }
}

component Chips() {
    view {
        <div>
            <Chip label="one" />
            <Chip label="two" tone={"warn".to_string()} loud={true} />
        </div>
    }
}
