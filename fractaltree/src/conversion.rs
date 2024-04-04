use super::*;

// Conversion impl for FractalTree to FractalTreeRead
impl From<FractalTreeBuild> for FractalTreeRead
{
    fn from(mut tree: FractalTreeBuild) -> Self
    {
        tree.flush_all();
        FractalTreeRead { root: tree.root.into() }
    }
}

// Conversion impl for FractalTree to FractalTreeRead
impl From<FractalTreeRead> for FractalTreeDisk
{
    fn from(tree: FractalTreeRead) -> Self
    {
        FractalTreeDisk {
            root: tree.root.into(),
            start: 0,
            ..Default::default()
        }
    }
}

impl From<FractalTreeBuild> for FractalTreeDisk
{
    fn from(mut tree: FractalTreeBuild) -> Self
    {
        tree.flush_all();
        let root: NodeRead = tree.root.into();
        FractalTreeDisk {
            root: root.into(),
            ..Default::default()
        }
    }
}

// Conversion for Box<Node> to Box<NodeRead>
impl From<Box<Node>> for Box<NodeRead>
{
    fn from(node: Box<Node>) -> Self
    {
        Box::new((*node).into())
    }
}

// Conversion for Node to NodeRead
// Todo: conversion may benefit from bumpalo or arch?
impl From<Node> for NodeRead
{
    fn from(node: Node) -> Self
    {
        let Node {
            is_root,
            is_leaf,
            keys,
            children,
            values,
            buffer: _,
        } = node;

        let children = if children.is_some() {
            let children = children.unwrap();
            let mut new_children = Vec::with_capacity(children.len());
            for child in children {
                new_children.push(child.into());
            }
            Some(new_children)
        } else {
            None
        };

        NodeRead {
            is_root,
            is_leaf,
            keys,
            children,
            values,
        }
    }
}

// Conversion for Box<Node> to Box<NodeRead>
impl From<Box<NodeRead>> for Box<NodeDisk>
{
    fn from(node: Box<NodeRead>) -> Self
    {
        Box::new((*node).into())
    }
}

impl From<NodeRead> for NodeDisk
{
    fn from(node: NodeRead) -> Self
    {
        let NodeRead {
            is_root,
            is_leaf,
            keys,
            children,
            values,
        } = node;

        NodeDisk {
            is_root,
            is_leaf,
            state: None,
            keys: keys.into_vec(),
            children: children.map(|children| children.into_iter().map(|child| child.into()).collect()),
            values,
        }
    }
}
