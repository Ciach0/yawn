(comment) @comment

["component" "server" "fn" "pub" "view" "style" "if" "else" "for" "key"] @keyword

(component name: (component_name) @type)
(server_fn name: (fn_name) @function)
(param name: (param_name) @variable.parameter)
(param "mut" @keyword)
(param type: (rust_type) @type)
(component_tag) @type
(tag_name) @tag
(attribute_name) @attribute
(directive) @keyword
(attribute "=" @operator)

(string) @string
(string_content) @string
(escape) @string.escape
(interpolation ["{" "}"] @punctuation.special)

(expression ["{" "}"] @punctuation.special)
["<" ">" "</" "/>"] @punctuation.bracket
["(" ")" "{" "}"] @punctuation.bracket
