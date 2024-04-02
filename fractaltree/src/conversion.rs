use super::*;

// Conversion impl for FractalTree to FractalTreeRead
impl<K, V> From<FractalTreeBuild<K, V>> for FractalTreeRead<K, V>
where
    K: Key,
    V: Value,
{
    fn from(tree: FractalTreeBuild<K, V>) -> Self {
        FractalTreeRead {
            root: tree.root.into(),
        }
    }
}

// Conversion impl for FractalTree to FractalTreeRead
impl<K, V> From<FractalTreeRead<K, V>> for FractalTreeDisk<K, V>
where
    K: Key,
    V: Value,
{
    fn from(tree: FractalTreeRead<K, V>) -> Self {
        FractalTreeDisk {
            root: tree.root.into(),
            start: 0,
            ..Default::default()
        }
    }
}

impl<K, V> From<NodeRead<K, V>> for NodeDiskState<K, V>
where
    K: Key,
    V: Value,
{
    fn from(node: NodeRead<K, V>) -> Self {
        NodeDiskState::InMemory(Box::new(node.into()))
    }
}

impl<K, V> From<FractalTreeBuild<K, V>> for FractalTreeDisk<K, V>
where
    K: Key,
    V: Value,
{
    fn from(tree: FractalTreeBuild<K, V>) -> Self {
        let root: NodeRead<K, V> = tree.root.into();
        FractalTreeDisk {
            root: root.into(),
            ..Default::default()
        }
    }
}

// Conversion for Box<Node> to Box<NodeRead>
impl<K, V> From<Box<Node<K, V>>> for Box<NodeRead<K, V>>
where
    K: Key,
    V: Value,
{
    fn from(node: Box<Node<K, V>>) -> Self {
        Box::new((*node).into())
    }
}

// Conversion for Node to NodeRead
// Todo: conversion may benefit from bumpalo or arch?
impl<K, V> From<Node<K, V>> for NodeRead<K, V>
where
    K: Key,
    V: Value,
{
    fn from(node: Node<K, V>) -> Self {
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
impl<K, V> From<Box<NodeRead<K, V>>> for Box<NodeDisk<K, V>>
where
    K: Key,
    V: Value,
{
    fn from(node: Box<NodeRead<K, V>>) -> Self {
        Box::new((*node).into())
    }
}

impl<K, V> From<NodeRead<K, V>> for NodeDisk<K, V>
where
    K: Key,
    V: Value,
{
    fn from(node: NodeRead<K, V>) -> Self {
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
            keys: keys.into_vec(),
            children: children.map(|children| {
                children
                    .into_iter()
                    .map(|child| Box::new(NodeDiskState::InMemory(child.into())))
                    .collect()
            }),
            values,
        }
    }
}
