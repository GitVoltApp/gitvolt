import { registerTabSlot } from '../../app/slots';
import { FlyoutHost } from './FlyoutHost';

/** The left flyout's host, over every repository tab's center. */
const off = registerTabSlot('centerOverlay', 'flyout', FlyoutHost);
import.meta.hot?.dispose(off);
