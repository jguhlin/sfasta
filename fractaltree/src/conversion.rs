use super::*;

impl<K: Key, V: Value, const RANGE: bool> From<FractalTreeBuild<K, V, RANGE>>
    for FractalTreeDisk<K, V, RANGE>
{
    fn from(mut tree: FractalTreeBuild<K, V, RANGE>) -> Self
    {
        tree.flush_all();
        let root: NodeBuild<K, V, RANGE> = tree.root.into();
        FractalTreeDisk {
            root: root.into(),
            ..Default::default()
        }
    }
}

impl<K: Key, V: Value, const RANGE: bool> From<NodeBuild<K, V, RANGE>>
    for NodeDisk<K, V, RANGE>
{
    fn from(node: NodeBuild<K, V, RANGE>) -> Self
    {
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

impl<K: Key, V: Value, const RANGE: bool>
    From<Box<build::NodeBuild<K, V, RANGE>>>
    for Box<disk::NodeDisk<K, V, RANGE>>
{
    fn from(node: Box<build::NodeBuild<K, V, RANGE>>) -> Self
    {
        Box::new((*node).into())
    }
}
