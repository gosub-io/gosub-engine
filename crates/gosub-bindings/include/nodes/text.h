#ifndef GOSUB_API_NODES_TEXT_H
#define GOSUB_API_NODES_TEXT_H

#include <stdbool.h>
#include <stdint.h>

struct node_t;

struct node_text_t {
  // this tag is not used but is required to map properly from Rust
  uint32_t tag;
  char *value;
  char *font;
  double font_size;
  bool is_bold;
};

void rendertree_node_text_free_data(struct node_text_t *text);

const char *rendertree_node_text_get_value(const struct node_t *node);
const char *rendertree_node_text_get_font(const struct node_t *node);
double rendertree_node_text_get_font_size(const struct node_t *node);
bool rendertree_node_text_get_bold(const struct node_t *node);

#endif
