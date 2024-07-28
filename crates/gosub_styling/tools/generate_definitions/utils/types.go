package utils

type Data struct {
	Properties []Properties `json:"properties"`
	Values     []Value      `json:"values"`
	AtRules    []AtRule     `json:"atrules"`
	Selectors  []Selector   `json:"selectors"`
}

type Value struct {
	Name   string `json:"name"`
	Syntax string `json:"syntax"`
}

type Properties struct {
	Name      string           `json:"name"`
	Syntax    string           `json:"syntax"`
	Computed  []string         `json:"computed"`
	Initial   StringMaybeArray `json:"initial"`
	Inherited bool             `json:"inherited"`
}

type AtRule struct {
	Name        string `json:"name"`
	Descriptors []struct {
		Name    string `json:"name"`
		Syntax  string `json:"syntax"`
		Initial string `json:"initial"`
	} `json:"descriptors"`
	Values []struct {
		Name   string `json:"name"`
		Value  string `json:"value,omitempty"`
		Values []struct {
			Name  string `json:"name"`
			Value string `json:"value"`
		}
	}
}

type Selector struct {
	Name string `json:"name"`
}
