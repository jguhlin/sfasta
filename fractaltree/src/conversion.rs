use super::*;

impl<K: Key, V: Value> From<FractalTreeBuild<K, V>> for FractalTreeDisk<K, V> {
    fn from(mut tree: FractalTreeBuild<K, V>) -> Self {
        tree.flush_all();
        let root: NodeBuild<K, V> = tree.root.into();
        FractalTreeDisk {
            root: root.into(),
            ..Default::default()
        }
    }
}

impl<K: Key, V: Value> From<NodeBuild<K, V>> for NodeDisk<K, V> {
    fn from(node: NodeBuild<K, V>) -> Self {
        let NodeBuild {
            is_root,
            is_leaf,
            keys,
            children,
            values,
            buffer,
        } = node;

        assert!(buffer.is_empty());

        NodeDisk {
            is_root,
            is_leaf,
            state: NodeState::InMemory,
            keys: keys.into_vec(),
            children: children.map(|children| {
                children.into_iter().map(|child| child.into()).collect()
            }),
            values,
        }
    }
}

impl<K: Key, V: Value> From<Box<build::NodeBuild<K, V>>>
    for Box<disk::NodeDisk<K, V>>
{
    fn from(node: Box<build::NodeBuild<K, V>>) -> Self {
        Box::new((*node).into())
    }
}
