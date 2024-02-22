pub struct BPlusTree<K, V> {
    root: Option<Box<Node<K, V>>>,
    order: u8,
}

impl<K, V> BPlusTree<K, V> {
    pub fn new(order: u8) -> Self {
        BPlusTree { root: None, order }
    }

    pub fn insert(&mut self, key: K, value: V)
    where
        K: PartialOrd + PartialEq,
    {
        if self.root.is_none() {
            self.root = Some(Box::new(Node::leaf(self.order)));
        }

        let mut root = self.root.take().unwrap();
        if root.keys.len() == 2 * self.order as usize - 1 {
            let (split_key, new_node) = root.split();
            let mut new_root = Box::new(Node::root(self.order));
            new_root.keys.push(split_key);
            new_root.children.as_mut().unwrap().push(root);
            new_root.children.as_mut().unwrap().push(new_node);
            root = new_root;
        }
        root.insert(key, value);
        self.root = Some(root);
    }
}

pub struct Node<K, V> {
    pub is_leaf: bool,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<Node<K, V>>>>,
    pub values: Option<Vec<V>>,
    pub next: Option<Box<Node<K, V>>>, // Does this need to be a u64 until loaded?

    // TODO: don't serialize this...
    pub order: u8,
}

impl<K, V> Node<K, V> {
    pub fn root(order: u8) -> Self {
        Node {
            is_leaf: false,
            keys: Vec::new(),
            children: Some(Vec::new()),
            values: None,
            next: None,
            order,
        }
    }

    pub fn leaf(order: u8) -> Self {
        Node {
            is_leaf: true,
            keys: Vec::new(),
            children: None,
            values: Some(Vec::new()),
            next: None,
            order,
        }
    }

    pub fn insert(&mut self, key: K, value: V)
    where
        K: PartialOrd + PartialEq,
    {
        if self.is_leaf {
            self.keys.push(key);
            self.values.as_mut().unwrap().push(value);

            // Sort keys and values
            let mut i = self.keys.len() - 1;
            while i > 0 && self.keys[i] < self.keys[i - 1] {
                self.keys.swap(i, i - 1);
                self.values.as_mut().unwrap().swap(i, i - 1);
                i -= 1;
            }
        } else {

            // Find the child we should insert into
            let mut i = 0;
            while i < self.keys.len() && key > self.keys[i] {
                i += 1;
            }
            self.children.as_mut().unwrap()[i].insert(key, value);

            // Split if necessary
            if self.children.as_ref().unwrap()[i].needs_split() {
                let (split_key, new_node) = self.children.as_mut().unwrap()[i].split();
                self.keys.insert(i, split_key);
                self.children.as_mut().unwrap().insert(i + 1, new_node);
            }
        }
    }
            
    pub fn split(&mut self) -> (K, Box<Node<K, V>>) {
        let mid = self.keys.len() / 2;

        let values = if self.values.is_some() {
            Some(self.values.as_mut().unwrap().split_off(mid))
        } else {
            None
        };

        let children = if self.children.is_some() {
            Some(self.children.as_mut().unwrap().split_off(mid))
        } else {
            None
        };

        let mut new_node = Box::new(Node {
            is_leaf: self.is_leaf,
            keys: self.keys.split_off(mid),
            children,
            values,
            next: self.next.take(),
            order: self.order,
        });
        (new_node.keys.pop().unwrap(), new_node)
    }

    pub fn needs_split(&self) -> bool {
        self.keys.len() == 2 * self.order as usize - 1
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn simple_addition() {
        let mut tree = super::BPlusTree::new(3);
        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.insert(3, "three");
        tree.insert(4, "four");
        tree.insert(5, "five");
        tree.insert(6, "six");
        tree.insert(7, "seven");
        tree.insert(8, "eight");
        tree.insert(9, "nine");
        tree.insert(10, "ten");
    }
}
