#ifndef GOSUB_API_H
#define GOSUB_API_H

#include "nodes.h"

extern void *gosub_rendertree_init(const char *html);
extern void *gosub_rendertree_iterator_init(void *rendertree);
extern const void *gosub_rendertree_next_node(void *tree_iterator);
extern void gosub_rendertree_get_node_data(const void *current_node,
                                            struct node_t *node);
extern void gosub_rendertree_iterator_free(void *tree_iterator);
extern void gosub_rendertree_free(void *render_free);

struct rendertree_t {
  void *tree;
  void *iterator;
  const void *current_node;
  struct node_t *data;
};

/// Initialize a render tree by passing a stack-allocated
/// struct by address.
/// Returns 0 on success or -1 if a failure occurred.
int8_t rendertree_init(struct rendertree_t *rendertree, const char *html);

/// Get the next node in the render tree as a read-only pointer.
/// Returns NULL when reaching end of tree.
const struct node_t *rendertree_next(struct rendertree_t *rendertree);

/// Get the type of the current node the render tree is pointing to.
enum node_type_e
rendertree_get_current_node_type(const struct rendertree_t *rendertree);

/// Free all memory tied to the render tree
void rendertree_free(struct rendertree_t *rendertree);

#endif
