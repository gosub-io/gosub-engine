#ifndef GOSUB_API_NODES_H
#define GOSUB_API_NODES_H

#include <stdbool.h>
#include <stddef.h> // for NULL (which is basically just 0... but more clear)
#include <stdint.h>
#include <stdlib.h>

#include "nodes/text.h"
#include "properties.h"

struct node_t *rendertree_node_init();
void rendertree_node_free(struct node_t **node);

enum node_type_e { NODE_TYPE_ROOT = 0u, NODE_TYPE_TEXT };

struct node_t {
  enum node_type_e type;
  struct position_t position;
  struct rectangle_t margin;
  struct rectangle_t padding;
  union data {
    bool root;               // NODE_TYPE_ROOT
    struct node_text_t text; // NODE_TYPE_TEXT
  } data;
};

struct node_t *rendertree_node_init();
double rendertree_node_get_x(const struct node_t *node);
double rendertree_node_get_y(const struct node_t *node);
double rendertree_node_get_margin_top(const struct node_t *node);
double rendertree_node_get_margin_left(const struct node_t *node);
double rendertree_node_get_margin_right(const struct node_t *node);
double rendertree_node_get_margin_bottom(const struct node_t *node);
double rendertree_node_get_padding_top(const struct node_t *node);
double rendertree_node_get_padding_left(const struct node_t *node);
double rendertree_node_get_padding_right(const struct node_t *node);
double rendertree_node_get_padding_bottom(const struct node_t *node);
void rendertree_node_free_data(struct node_t *node);
void rendertree_node_free(struct node_t **node);

#endif
