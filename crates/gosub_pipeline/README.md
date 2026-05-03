# Render pipeline POC

This repo contains a proof of concept for a render pipeline that can be used to render a webpage.

The pipeline consists of multiple stages. Each stage is responsible for transforming the data from the previous stage into a new format or 
updates on current data. Depending on any changes in the data, the pipeline can be re-run to update the rendering. It's possible that not 
the whole pipeline needs to be re-run, but only a subset of the stages.

The stages:

 - Rendertree generation - Convert the DOM tree into a render tree
 - Layout tree generation - Computing the layout of the elements
 - Layering - Grouping elements into layers
 - Tiling - Splitting the layout tree into tiles
 - Painting - Generating paint commands
 - Rasterizing - Executing paint commands onto tiles
 - Compositing - combine the tiles into a final image

The first step is generating the render tree. It will convert a DOM tree together with CSS styles into a tree of nodes that are needed for 
generating layout. The node IDs in the render-tree are the same as the node IDs in the DOM tree and are interchangable: NodeId(4) is the 
same as RenderNodeId(4) though they have different types. 

The second step is to generate a layout. For this we use Taffy to compute all the layout elements and use pango for generating text layout.
The layout elements are the building blocks for the layout tree. To make sure we are not dependend on taffy, the output of the layout tree is a 
BoxModel system, where each element is confined into a box model. This boxmodel holds the dimensions of the margin, padding, border and content.

The third step is to generate layers. Layers are used to optimize rendering. They are used to group elements that can be rendered together.
If there are elements with some kind of CSS animations, they can be moved to a separate layer, and let the compositor deal with this animation.
This means that we do not need to rerender the layers or tiles, but merely update the position of the layers in the compositor. As a demonstration,
we place all elements in layer 0, and place images inside layer 1.

The next step is tiling. Here we convert the layout tree into elements of 256x256 pixels (tiles). This is done to optimize rendering dirty elements. 
Only the tiles that are visible on the screen are rendered and cached. When the user scrolls, we only need to render the new tiles that are visible 
on the screen. This however, can be done during idle time in the browser as well. Furthermore, if the user scrolls backwards, older tiles that are
still valid do not have to be rendered again.

The painting generates commands that are needed to render pixels onto the tiles. However, it does not execute this painting. It merely generates
the commands.

The rastering phase will get the tiles and the paint commands and execute the painting per tile into textures.

The final step is compositing. Here we combine the visible tiles in the layers onto the screen. When we have CSS animations like transitions, we
do not need to repaint the tiles, but merely update the position of the tiles (or their opacity). The compositing will take care of this and returns 
fully rendered frame.


## Passing of data
Each stage will take the data from the previous stage and transform it into a new format. Note that the data from earlier stages are still available 
by wrapping these structures.

For instance, the layering stage will take the layout tree and the render tree as input. The output of the layering stage is a list of layers.
Note that we have a wrapped layout tree, which in turn has a wrapped render tree which in turn has a wrapped DOM document.


# Data structure throughout the pipelines
Note that each structure wraps the previous structure so it's always possible to look back into the previous stage for information. Normally,
the layout list and dom nodes are important for later stages.

```
TileList
    - layers: HashMap<LayerId, Vec<TileId>>
    - default_tile_width
    - default_tile_height
    - wrapped[layer_list]
        - layers: Vec<Layer>
                - id: LayerId
                - order: isize
                - elements: Vec<NodeId>
        - wrapped[layout_tree]
            - taffy_tree
            - taffy_root_id
            - root_layout_element: LayoutElementNode
                - node_id: LayoutElementId
                - dom_node_id: DomNodeId
                - taffy_node_id: TaffyNodeId
                - children: Vec<LayoutElementNode>
                - box_model: BoxModel
            - node_mapping
            - wrapped[render_tree]: RenderTree
                - root: RenderNode
                    - node_id: NodeId
                    - children: Vec<RenderNode>
                - wrapped[doc]: Document
                    - root: Node
                        - node_id: NodeId
                        - children: Vec<Node>
                        - node_type: NodeType
```


# Directory layout
Each stage has its own file in the `src` directory. The `main.rs` file contains the main function that runs the pipeline. If a stage is larger (most of them are), it will 
be split into a module with the corresponding name. Some code is shared between stages (geometry, document, texture and images stores etc), and they are placed into the 
`common` module. Note that it's possible for any pipeline module to use the `common` module, but not the other way around.


# Main demo applications
There are three demo applications, each running with its own backend.

| Binary         | 2d rendering lib | text rendering lib |
|----------------|------------------|--------------------|
| pipeline-cairo | `cairo`            | `pangocairo`         |
| pipeline-vello | `vello`            | `parley` or `skia`     |
| pipeline-skia  | `skia`             | `skia`               |


# Media store
The media store is a simple in-memory store that keeps external (or inline) resources. It's used for storing images and SVG files but it allows to store 
any kind of data. This media-store can be an offline cache for resources in the future. 


# Texture store
The texture store keeps all the textures from page tiles. This way we only need to rerender tiles when elements on them are dirty. For scrolling and other 
purposes, we do not need to rerender the tiles but the compositor can take care of this. Even though we can use GPU accelerated rendering, we still need to 
store the textures in memory. A good optimization might be to store the textures on the GPU and reference them in the texture store.

# Rstar
This pipeline relies on rstar for spatial searched. For instance, we need to know which elements are visible on the screen. Or which elements are at a certain position.
Some of the pipeline data structures will have a separate rstar tree for this purpose. 