package utils

type Data struct {
	Properties  []Property  `json:"properties"`
	Values      []Value     `json:"values"`
	AtRules     []AtRule    `json:"atrules"`
	Selectors   []Selector  `json:"selectors"`
	PropAliases []PropAlias `json:"propAliases"`
}

type PropAlias struct {
	Name string `json:"name"`
	For  string `json:"for"`
}

type Value struct {
	Name   string `json:"name"`
	Syntax string `json:"syntax"`
}

type Property struct {
	Name      string           `json:"name"`
	Syntax    string           `json:"syntax"`
	Computed  []string         `json:"computed"`
	Initial   StringMaybeArray `json:"initial"`
	Inherited bool             `json:"inherited"`
}

type AtRule struct {
	Name        string             `json:"name"`
	Descriptors []AtRuleDescriptor `json:"descriptors"`
	Values      []struct {
		Name   string `json:"name"`
		Value  string `json:"value,omitempty"`
		Values []struct {
			Name  string `json:"name"`
			Value string `json:"value"`
		}
	}
}

type AtRuleDescriptor struct {
	Name    string `json:"name"`
	Syntax  string `json:"syntax"`
	Initial string `json:"initial"`
}

type Selector struct {
	Name string `json:"name"`
}
