#include "gosub_api.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

int main() {
  const char *html = "<html>"
                     "<h1>this is heading 1</h1>"
                     "<h2>this is heading 2</h2>"
                     "<h3>this is heading 3</h3>"
                     "<h4>this is heading 4</h4>"
                     "<h5>this is heading 5</h5>"
                     "<h6>this is heading 6</h6>"
                     "<p>this is a paragraph</p>"
                     "</html>";
  struct render_tree_t *render_tree = render_tree_init(html);
  assert(render_tree != NULL);

  const struct node_t *node = NULL;

  // <html>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_ROOT);

  // <h1>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 1") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 37.0) < 0.00001);
  assert(node->data.text.is_bold == true);

  // <h2>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 2") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 27.5) < 0.00001);
  assert(node->data.text.is_bold == true);

  // <h3>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 3") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 21.5) < 0.00001);
  assert(node->data.text.is_bold == true);

  // <h4>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 4") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 18.5) < 0.00001);
  assert(node->data.text.is_bold == true);

  // <h5>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 5") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 15.5) < 0.00001);
  assert(node->data.text.is_bold == true);

  // <h6>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 6") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 12.0) < 0.00001);
  assert(node->data.text.is_bold == true);

  // <p>
  node = render_tree_next(render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is a paragraph") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 18.5) < 0.00001);
  assert(node->data.text.is_bold == false);

  // end of iterator, last node is free'd
  node = render_tree_next(render_tree);
  assert(node == NULL);

  render_tree_free(&render_tree);

  printf("\033[0;32mrender_tree_test.c: All assertions passed\n\033[0;30m");
  return 0;
}
